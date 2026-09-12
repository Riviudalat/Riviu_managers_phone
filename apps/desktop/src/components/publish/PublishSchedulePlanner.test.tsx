import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { publishScheduleCreate, publishSchedulePreflight } from "../../api";
import type { DeviceInfo, PublishBundle, PublishScheduleReport } from "../../types";
import { PublishSchedulePlanner } from "./PublishSchedulePlanner";
import { SCHEDULE_DRAFT_KEY } from "./publishScheduleAllocation";
vi.mock("../../api", () => ({ publishScheduleCreate: vi.fn(), publishSchedulePreflight: vi.fn(), publishImagePreview: vi.fn() }));
const bundles: PublishBundle[] = Array.from({ length: 3 }, (_, i) => ({ id: `b${i}`, name: `Bài ${i + 1}`, sourcePath: `C:/posts/${i}`, mediaKind: "image", images: [], captionPath: "caption.txt", caption: `Caption ${i}`, captionSha256: "a".repeat(64), totalBytes: 100 }));
const devices: DeviceInfo[] = Array.from({ length: 3 }, (_, i) => ({ udid: `phone-${i + 1}`, name: "Android", model: "SM-G955N", platform: "android", osVersion: "9", connection: "usb", status: "ready", wdaReady: true }));
const props = { sourceRoot: "C:/posts", bundles, devices, metas: new Map(), captions: {}, sound: { kind: "default" as const }, sheet: false, cleanup: false, onCreated: vi.fn(), onSource: vi.fn() };
const report = (count = 3): PublishScheduleReport => ({ inputDigest: "approved", canExecute: true, slots: Array.from({ length: count }, () => ({ inputDigest: "slot", canExecute: true, issues: [], assignments: [], sheetConfigured: false, targetSnapshot: { targetRef: { type: "explicit", udids: [] }, included: [], excluded: [], rosterSha256: "hash" } })) });
beforeEach(() => { localStorage.clear(); vi.clearAllMocks(); vi.mocked(publishSchedulePreflight).mockImplementation(async request => report(request.slots.length)); vi.mocked(publishScheduleCreate).mockResolvedValue([{ id: "one" }, { id: "two" }, { id: "three" }] as never); });
afterEach(() => { cleanup(); vi.restoreAllMocks(); });
async function assignAll() {
  await userEvent.click(screen.getByRole("button", { name: "Chọn tất cả bài" }));
  await userEvent.click(screen.getByRole("button", { name: "Chọn tất cả sẵn sàng" }));
  await userEvent.click(screen.getByRole("button", { name: "Gán bài đã chọn" }));
  fireEvent.change(screen.getByLabelText("Ngày đăng"), { target: { value: "2099-09-10" } });
  fireEvent.change(screen.getByLabelText("Giờ chung"), { target: { value: "20:00" } });
}
describe("schedule assignment workspace", () => {
  it("quick-selects ten posts onto ten machines and expands when more phones are ready", async () => {
    const ten = Array.from({ length: 10 }, (_, i) => ({ ...bundles[0], id: `q${i}`, name: `Quick ${i}` }));
    const twelve = Array.from({ length: 12 }, (_, i) => ({ ...devices[0], udid: `m${i}` }));
    const view = render(<PublishSchedulePlanner {...props} bundles={ten} devices={twelve.slice(0, 10)} />);
    await userEvent.click(screen.getByRole("button", { name: "Chọn nhanh" }));
    const original = localStorage.getItem(SCHEDULE_DRAFT_KEY)!;
    const saved = JSON.parse(original);
    expect(saved.rows.map((r: { udid: string }) => r.udid)).toEqual(twelve.slice(0, 10).map(d => d.udid));
    expect(saved.selectedMachines).toHaveLength(10);
    expect(saved.commonTime).toBe("");
    expect(screen.getByRole("status")).toHaveTextContent("Đã gán 10 bài cho 10 máy");
    fireEvent.click(screen.getByRole("button", { name: "Chọn nhanh" }));
    expect(localStorage.getItem(SCHEDULE_DRAFT_KEY)).toBe(original);
    const more = Array.from({ length: 14 }, (_, i) => ({ ...bundles[0], id: `q${i}`, name: `Quick ${i}` }));
    view.rerender(<PublishSchedulePlanner {...props} bundles={more} devices={twelve} />);
    await userEvent.click(screen.getByRole("button", { name: "Chọn nhanh" }));
    const expanded = JSON.parse(localStorage.getItem(SCHEDULE_DRAFT_KEY)!);
    expect(expanded.rows).toHaveLength(12);
    expect(expanded.selectedMachines).toHaveLength(12);
    await userEvent.click(screen.getByRole("button", { name: "Hoàn tác" }));
    expect(JSON.parse(localStorage.getItem(SCHEDULE_DRAFT_KEY)!).rows).toHaveLength(10);
    expect(publishSchedulePreflight).not.toHaveBeenCalled(); expect(publishScheduleCreate).not.toHaveBeenCalled();
  });
  it("quick selection keeps manual mappings and fills free ready machines for the current rows", async () => {
    render(<PublishSchedulePlanner {...props} selectedIds={["b0", "b1"]} assignments={{ b0: "phone-2" }} />);
    await userEvent.click(screen.getByLabelText("Chọn máy hẹn giờ Máy 3"));
    await userEvent.click(screen.getByRole("button", { name: "Chọn nhanh" }));
    expect(screen.getByLabelText("Máy nhận Bài 1")).toHaveValue("phone-2");
    expect(screen.getByLabelText("Máy nhận Bài 2")).toHaveValue("phone-1");
    expect(screen.queryByLabelText("Máy nhận Bài 3")).toBeNull();
    expect(screen.getByLabelText("Chọn máy hẹn giờ Máy 1")).toBeChecked();
    await userEvent.click(screen.getByRole("button", { name: "Chọn nhanh" }));
    expect(screen.getByLabelText("Máy nhận Bài 3")).toHaveValue("phone-3");
  });
  it("keeps lost machines out and fills the remaining ready capacity", async () => {
    const view = render(<PublishSchedulePlanner {...props} />);
    await userEvent.click(screen.getByLabelText("Chọn máy hẹn giờ Máy 1"));
    await userEvent.click(screen.getByLabelText("Chọn máy hẹn giờ Máy 2"));
    view.rerender(<PublishSchedulePlanner {...props} devices={devices.filter(d => d.udid !== "phone-2")} />);
    await userEvent.click(screen.getByRole("button", { name: "Chọn nhanh" }));
    expect(screen.getByRole("status")).toHaveTextContent("Đã gán 2 bài cho 2 máy");
    expect(screen.getByLabelText("Máy nhận Bài 1")).toHaveValue("phone-1");
    expect(screen.getByLabelText("Máy nhận Bài 2")).toHaveValue("phone-3");
    expect(screen.queryByLabelText("Máy nhận Bài 3")).toBeNull();
    expect(screen.getByRole("button", { name: "Kiểm tra lịch" })).toBeDisabled();
    await userEvent.click(screen.getByRole("button", { name: "Đặt giờ" }));
    expect(screen.getByLabelText("Giờ chung")).toHaveFocus();
  });
  it("keeps draft storage errors visible after successful assignment and allows retry", async () => {
    const write = vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw new Error("disk full"); });
    render(<PublishSchedulePlanner {...props} />);
    await userEvent.click(screen.getByRole("button", { name: "Chọn nhanh" }));
    expect(screen.getByRole("alert")).toHaveTextContent("Chưa lưu được bản nháp lịch");
    expect(screen.getByRole("status")).toHaveTextContent("Đã gán 3 bài cho 3 máy");
    fireEvent.change(screen.getByLabelText("Giờ chung"), { target: { value: "20:00" } });
    expect(screen.getByRole("alert")).toBeVisible();
    write.mockRestore();
    await userEvent.click(screen.getByRole("button", { name: "Thử lưu nháp lại" }));
    expect(screen.queryByRole("alert")).toBeNull();
    expect(JSON.parse(localStorage.getItem(SCHEDULE_DRAFT_KEY)!).rows).toHaveLength(3);
  });
  it("guides date, Sheet and confirmation focus without creating a schedule", async () => {
    const onSheetSetup = vi.fn();
    const view = render(<PublishSchedulePlanner {...props} onSheetSetup={onSheetSetup} blockingReason="Kiểm tra Sheet" />);
    await userEvent.click(screen.getByRole("button", { name: "Chọn nhanh" }));
    await userEvent.click(screen.getByRole("button", { name: "Đặt giờ" }));
    expect(screen.getByLabelText("Giờ chung")).toHaveFocus();
    fireEvent.change(screen.getByLabelText("Ngày đăng"), { target: { value: "2099-09-10" } });
    fireEvent.change(screen.getByLabelText("Giờ chung"), { target: { value: "20:00" } });
    await userEvent.click(screen.getByRole("button", { name: "Kiểm tra Sheet" }));
    expect(onSheetSetup).toHaveBeenCalledOnce();
    view.rerender(<PublishSchedulePlanner {...props} onSheetSetup={onSheetSetup} />);
    await userEvent.click(screen.getByRole("button", { name: "Kiểm tra lịch" }));
    await userEvent.click(screen.getByRole("button", { name: "Tới xác nhận" }));
    expect(screen.getByRole("checkbox", { name: /Tôi xác nhận/ })).toHaveFocus();
    await userEvent.click(screen.getByRole("checkbox", { name: /Tôi xác nhận/ }));
    expect(screen.getByRole("button", { name: "Lưu lịch 3 bài" })).toBeEnabled();
    await userEvent.click(screen.getByLabelText("Gỡ gán Bài 1"));
    await userEvent.click(screen.getByRole("button", { name: "Chọn nhanh" }));
    expect(screen.getByRole("button", { name: "Lưu lịch 3 bài" })).toBeDisabled();
    expect(publishScheduleCreate).not.toHaveBeenCalled();
  });
  it("highlights the matching assignment and clears an empty machine search", async () => {
    render(<PublishSchedulePlanner {...props} />);
    await userEvent.click(screen.getByRole("button", { name: "Chọn nhanh" }));
    fireEvent.focus(screen.getByRole("button", { name: "Kéo bài Bài 2" }));
    expect(document.querySelector('[data-schedule-device="phone-2"]')).toHaveClass("is-linked");
    expect(document.querySelector('[data-schedule-row="b1"]')).toHaveClass("is-linked");
    fireEvent.change(screen.getByLabelText("Tìm máy hẹn giờ"), { target: { value: "nothing" } });
    expect(screen.getByText("Không có máy khớp từ khóa.")).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: "Xóa tìm kiếm" }));
    expect(screen.getByLabelText("Tìm máy hẹn giờ")).toHaveValue("");
    expect(screen.getByLabelText("Tìm máy hẹn giờ")).toHaveFocus();
  });
  it("waits for first activation to seed Setup choices and never overwrites its own draft", async () => {
    const view = render(<PublishSchedulePlanner {...props} active={false} selectedIds={[]} />);
    view.rerender(<PublishSchedulePlanner {...props} active selectedIds={["b0", "b1"]} assignments={{ b0: "phone-2" }} />);
    expect(screen.getByLabelText("Máy nhận Bài 1")).toHaveValue("phone-2");
    expect(screen.getByLabelText("Giờ chung")).toHaveValue("");
    view.rerender(<PublishSchedulePlanner {...props} active={false} selectedIds={["b2"]} />);
    view.rerender(<PublishSchedulePlanner {...props} active selectedIds={["b2"]} />);
    expect(screen.getByLabelText("Chọn bài Bài 1")).toBeChecked();
    expect(screen.getByLabelText("Chọn bài Bài 3")).not.toBeChecked();
    expect(publishScheduleCreate).not.toHaveBeenCalled();
  });
  it("uses one common time and creates only after a current review and explicit confirmation", async () => {
    render(<PublishSchedulePlanner {...props} />); await assignAll();
    expect(screen.getByRole("button", { name: "Lưu lịch 3 bài" })).toBeDisabled();
    await userEvent.click(screen.getByRole("button", { name: "Kiểm tra lịch" }));
    const request = vi.mocked(publishSchedulePreflight).mock.calls[0][0];
    expect(request.slots.map(s => s.runAt)).toEqual(Array(3).fill("2099-09-10T20:00"));
    expect(new Set(request.slots.map(s => s.udid)).size).toBe(3);
    expect(publishScheduleCreate).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("checkbox", { name: /Tôi xác nhận/ }));
    await userEvent.dblClick(screen.getByRole("button", { name: "Lưu lịch 3 bài" }));
    expect(publishScheduleCreate).toHaveBeenCalledExactlyOnceWith(request, "approved", true);
    expect(props.onCreated).toHaveBeenCalledOnce();
  });
  it("preserves individual times across common-time changes and reload", async () => {
    const view = render(<PublishSchedulePlanner {...props} />); await assignAll();
    await userEvent.click(screen.getByLabelText("Chỉnh giờ từng bài"));
    fireEvent.change(screen.getByLabelText("Giờ bài Bài 2"), { target: { value: "21:00" } });
    fireEvent.change(screen.getByLabelText("Giờ chung"), { target: { value: "22:00" } });
    expect(screen.getByLabelText("Giờ bài Bài 1")).toHaveValue("22:00");
    expect(screen.getByLabelText("Giờ bài Bài 2")).toHaveValue("21:00");
    await userEvent.click(screen.getByRole("button", { name: "Kiểm tra lịch" }));
    view.unmount(); render(<PublishSchedulePlanner {...props} />);
    expect(screen.getByLabelText("Giờ bài Bài 2")).toHaveValue("21:00");
    expect(screen.getByRole("button", { name: "Lưu lịch 3 bài" })).toBeDisabled();
    await userEvent.click(screen.getByLabelText("Dùng giờ chung cho Bài 2"));
    expect(screen.getByLabelText("Giờ bài Bài 2")).toHaveValue("22:00");
  });
  it("restores three legacy rows on one phone at distinct times without changing their identity", async () => {
    const rows = bundles.map((b, i) => ({ id: `old${i}`, bundleId: b.id, udid: "phone-1", time: `20:${String(i * 5).padStart(2, "0")}` }));
    localStorage.setItem(SCHEDULE_DRAFT_KEY, JSON.stringify({ sourceRoot: props.sourceRoot, date: "2099-09-10", requestId: "old-id", rows }));
    render(<PublishSchedulePlanner {...props} selectedIds={["b2"]} />);
    expect(screen.getByLabelText("Giờ bài Bài 2")).toHaveValue("20:05");
    await userEvent.click(screen.getByRole("button", { name: "Kiểm tra lịch" }));
    expect(vi.mocked(publishSchedulePreflight).mock.calls[0][0]).toMatchObject({ requestId: "old-id", slots: rows.map(r => ({ bundleId: r.bundleId, udid: r.udid, runAt: `2099-09-10T${r.time}` })) });
  });
  it("partially assigns available machines, rejects occupied destinations and undoes only the last edit", async () => {
    render(<PublishSchedulePlanner {...props} devices={devices.slice(0, 2)} />); await assignAll();
    expect(screen.getByLabelText("Máy nhận Bài 3")).toHaveValue("");
    expect(screen.getByRole("button", { name: "Kiểm tra lịch" })).toBeDisabled();
    await userEvent.selectOptions(screen.getByLabelText("Máy nhận Bài 3"), "phone-1");
    expect(screen.getByLabelText("Máy nhận Bài 1")).toHaveValue("phone-1");
    expect(screen.getByLabelText("Máy nhận Bài 3")).toHaveValue("");
    await userEvent.click(screen.getByLabelText("Gỡ gán Bài 1"));
    expect(screen.getByLabelText("Máy nhận Bài 2")).toHaveValue("phone-2");
    await userEvent.click(screen.getByRole("button", { name: "Hoàn tác" }));
    expect(screen.getByLabelText("Máy nhận Bài 1")).toHaveValue("phone-1");
  });
  it("invalidates approval on disconnect, past time or conflicting custom time", async () => {
    const view = render(<PublishSchedulePlanner {...props} />); await assignAll();
    await userEvent.click(screen.getByRole("button", { name: "Kiểm tra lịch" }));
    await userEvent.click(screen.getByRole("checkbox", { name: /Tôi xác nhận/ }));
    view.rerender(<PublishSchedulePlanner {...props} eligible={["phone-1", "phone-2"]} />);
    expect(screen.getByRole("button", { name: "Lưu lịch 3 bài" })).toBeDisabled();
    view.rerender(<PublishSchedulePlanner {...props} />);
    expect(screen.getByRole("button", { name: "Lưu lịch 3 bài" })).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Ngày đăng"), { target: { value: "2000-01-01" } });
    expect(screen.getByRole("button", { name: "Kiểm tra lịch" })).toBeDisabled();
    expect(publishScheduleCreate).not.toHaveBeenCalled();
  });
  it("rejects late checks after the Sheet gate changes even if it becomes ready again", async () => {
    let release!: (r: PublishScheduleReport) => void;
    vi.mocked(publishSchedulePreflight).mockImplementationOnce(() => new Promise(r => { release = r; }));
    const view = render(<PublishSchedulePlanner {...props} />); await assignAll();
    await userEvent.click(screen.getByRole("button", { name: "Kiểm tra lịch" }));
    view.rerender(<PublishSchedulePlanner {...props} blockingReason="Kiểm tra Sheet" />);
    view.rerender(<PublishSchedulePlanner {...props} />);
    await act(async () => release(report()));
    expect(screen.queryByRole("checkbox", { name: /Tôi xác nhận/ })).toBeNull();
    expect(screen.getByRole("button", { name: "Lưu lịch 3 bài" })).toBeDisabled();
  });
  it("blocks Sheet before checking and ignores an unmounted check reply", async () => {
    const view = render(<PublishSchedulePlanner {...props} blockingReason="Kiểm tra Sheet" />); await assignAll();
    expect(screen.getByRole("button", { name: "Kiểm tra lịch" })).toBeDisabled();
    expect(publishSchedulePreflight).not.toHaveBeenCalled();
    view.rerender(<PublishSchedulePlanner {...props} />);
    let release!: (r: PublishScheduleReport) => void;
    vi.mocked(publishSchedulePreflight).mockImplementationOnce(() => new Promise(r => { release = r; }));
    await userEvent.click(screen.getByRole("button", { name: "Kiểm tra lịch" }));
    view.unmount(); await act(async () => release(report()));
    expect(publishScheduleCreate).not.toHaveBeenCalled();
  });
  it("caps selection at 100 and does not auto-assign an out-of-scope machine", async () => {
    const many = Array.from({ length: 101 }, (_, i) => ({ ...bundles[0], id: `b${i}`, name: `Bài ${i}` }));
    render(<PublishSchedulePlanner {...props} bundles={many} eligible={[]} />);
    await userEvent.click(screen.getByRole("button", { name: "Chọn nhanh" }));
    await waitFor(() => expect(document.querySelectorAll("[data-schedule-row]")).toHaveLength(0));
    expect(screen.getByRole("button", { name: "Gán bài đã chọn" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Kiểm tra lịch" })).toBeDisabled();
  });
  it("links a failed backend verdict to its exact row", async () => {
    const failed = report(); failed.canExecute = false; failed.slots[1].canExecute = false;
    failed.slots[1].issues = [{ code: "fixture", message: "Máy cần kiểm tra", severity: "error" }] as never;
    vi.mocked(publishSchedulePreflight).mockResolvedValueOnce(failed);
    render(<PublishSchedulePlanner {...props} />); await assignAll();
    await userEvent.click(screen.getByRole("button", { name: "Kiểm tra lịch" }));
    await userEvent.click(screen.getByRole("button", { name: "Xem lỗi" }));
    expect(document.querySelector('[data-schedule-row="b1"] [data-schedule-field="verdict"]')).toHaveFocus();
    expect(screen.getByRole("button", { name: "Lưu lịch 3 bài" })).toBeDisabled();
    expect(publishScheduleCreate).not.toHaveBeenCalled();
  });
  it("rechecks the roster at pointer release and commits two fast drops without sharing a machine", () => {
    const elementFromPoint = vi.fn();
    Object.defineProperty(document, "elementFromPoint", { configurable: true, value: elementFromPoint });
    Object.defineProperty(HTMLElement.prototype, "setPointerCapture", { configurable: true, value: vi.fn() });
    Object.defineProperty(HTMLElement.prototype, "hasPointerCapture", { configurable: true, value: () => false });
    const view = render(<PublishSchedulePlanner {...props} />);
    fireEvent.click(screen.getByRole("button", { name: "Chọn tất cả sẵn sàng" }));
    const zone = screen.getByRole("region", { name: "Vùng máy nhận bài" });
    const root = screen.getByRole("region", { name: "Lịch đăng nhiều khung giờ" });
    elementFromPoint.mockReturnValue(zone);
    const start = (name: string) => {
      fireEvent.pointerDown(screen.getByRole("button", { name: `Kéo bài ${name}` }), { button: 0, pointerId: 1, clientX: 10, clientY: 10 });
      fireEvent.pointerMove(root, { pointerId: 1, clientX: 80, clientY: 20 });
    };
    start("Bài 1");
    view.rerender(<PublishSchedulePlanner {...props} devices={devices.slice(1)} />);
    fireEvent.pointerUp(root, { pointerId: 1, clientX: 80, clientY: 20 });
    expect(screen.getByLabelText("Máy nhận Bài 1")).toHaveValue("phone-2");
    start("Bài 2"); fireEvent.pointerUp(root, { pointerId: 1, clientX: 80, clientY: 20 });
    expect(screen.getByLabelText("Máy nhận Bài 2")).toHaveValue("phone-3");
    expect(publishScheduleCreate).not.toHaveBeenCalled();
  });
});

it("invalidates the save button when its reviewed time passes without further edits", async () => {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(new Date("2099-09-10T19:59:00"));
  try {
    render(<PublishSchedulePlanner {...props}/>); await assignAll();
    await userEvent.click(screen.getByRole("button", { name: "Kiểm tra lịch" }));
    await userEvent.click(screen.getByRole("checkbox", { name: /Tôi xác nhận/ }));
    expect(screen.getByRole("button", { name: "Lưu lịch 3 bài" })).toBeEnabled();
    vi.setSystemTime(new Date("2099-09-10T20:00:01"));
    fireEvent(window,new Event("focus"));
    await waitFor(()=>expect(screen.getByRole("button", { name: "Lưu lịch 3 bài" })).toBeDisabled());
    expect(publishScheduleCreate).not.toHaveBeenCalled();
  } finally { vi.useRealTimers(); }
});
it("retains the reviewed request after a partial save reply for idempotent retry", async () => {
  vi.mocked(publishScheduleCreate).mockResolvedValueOnce([{ id: "partial" }] as never);
  render(<PublishSchedulePlanner {...props}/>); await assignAll();
  await userEvent.click(screen.getByRole("button", { name: "Kiểm tra lịch" }));
  await userEvent.click(screen.getByRole("checkbox", { name: /Tôi xác nhận/ }));
  await userEvent.click(screen.getByRole("button", { name: "Lưu lịch 3 bài" }));
  expect(document.querySelectorAll("[data-schedule-row]")).toHaveLength(3);
  expect(screen.getByRole("status")).toHaveTextContent("Kết quả lưu chưa khớp");
  expect(props.onCreated).not.toHaveBeenCalled();
  const first = vi.mocked(publishScheduleCreate).mock.calls[0][0];
  await userEvent.click(screen.getByRole("button", { name: "Lưu lịch 3 bài" }));
  expect(vi.mocked(publishScheduleCreate).mock.calls[1][0]).toEqual(first);
});

it("keeps a reviewed schedule when an unrelated device disconnects", async()=>{
 const view=render(<PublishSchedulePlanner {...props} selectedIds={["b0"]} assignments={{b0:"phone-1"}}/>);
 fireEvent.change(screen.getByLabelText("Ngày đăng"),{target:{value:"2099-09-10"}});
 fireEvent.change(screen.getByLabelText("Giờ chung"),{target:{value:"20:00"}});
 await userEvent.click(screen.getByRole("button",{name:"Kiểm tra lịch"}));
 await userEvent.click(screen.getByRole("checkbox",{name:/Tôi xác nhận/}));
 view.rerender(<PublishSchedulePlanner {...props} devices={devices.slice(0,2)}/>);
 expect(screen.getByRole("button",{name:"Lưu lịch 1 bài"})).toBeEnabled();
});
