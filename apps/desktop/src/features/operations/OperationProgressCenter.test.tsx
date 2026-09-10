import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { operationDeviceLog, operationGetRun, operationQueryRuns } from "../../api";
import type { OperationRunDetail, OperationRunSummary } from "../../types";
import { OperationProgressCenter } from "./OperationProgressCenter";
let narrowViewport = false;
vi.mock("../../useMediaQuery", () => ({ useMediaQuery: () => narrowViewport }));

vi.mock("../../api", () => ({ operationQueryRuns: vi.fn(), operationGetRun: vi.fn(), operationDeviceLog: vi.fn(), nurtureSessionStatus: vi.fn(async () => []) }));
const run: OperationRunSummary = { id: "publish:run", sourceId: "run", kind: "publish", title: "Đăng bài", state: "running", targetCount: 2, totalItems: 2, completedItems: 1, issueCount: 0, retryableCount: 0, retryScope: null, createdAt: null, updatedAt: null };
const detail: OperationRunDetail = { summary: run, items: ["a", "b"].map((udid, i) => ({ id: udid, udid, label: `Bài ${i + 1}`, kind: "assignment", state: i ? "running" : "succeeded", detail: null, errorCode: null, evidence: null, retryable: false })) };
const labels = new Map([["a", "Máy 2 · Nội dung"], ["b", "Máy 5 · Đăng bài"]]);
afterEach(cleanup);
beforeEach(() => {
  narrowViewport = false;
  localStorage.clear();
  vi.clearAllMocks();
  HTMLElement.prototype.setPointerCapture = vi.fn();
  vi.mocked(operationQueryRuns).mockResolvedValue({ runs: [run], total: 1, counts: { active: 1, succeeded: 0, attention: 0 }, hasMore: false });
  vi.mocked(operationGetRun).mockResolvedValue(detail);
  vi.mocked(operationDeviceLog).mockResolvedValue({ entries: [{ id: "log", at: "2026-09-07T12:34:56", action: "publish", state: "posting", text: null, detail: null }], truncated: false });
});

it("drags the title without a separate grip and does not toggle on release", async () => {
  await openDevices();
  expect(screen.queryByRole("button", { name: "Di chuyển cửa sổ tiến trình" })).not.toBeInTheDocument();
  const title = screen.getByRole("button", { name: "Tiến trình công việc" });
  const panel = screen.getByRole("dialog", { name: "Cửa sổ tiến trình" });
  const rect = vi.spyOn(panel, "getBoundingClientRect").mockReturnValue({ left: 300, top: 200, width: 760, height: 500 } as DOMRect);
  try {
    fireEvent.pointerDown(title, { button: 0, isPrimary: true, pointerId: 1, clientX: 360, clientY: 220 });
    fireEvent.pointerMove(title, { pointerId: 1, clientX: 200, clientY: 120 });
    fireEvent.pointerUp(title, { button: 0, pointerId: 1, clientX: 200, clientY: 120 });
    fireEvent.click(title, { detail: 1 });
    expect(panel).toHaveStyle({ left: "140px", top: "100px" });
    expect(screen.getByRole("dialog", { name: "Cửa sổ tiến trình" })).toBe(panel);
    fireEvent.pointerDown(title, { button: 0, isPrimary: true, pointerId: 2, clientX: 200, clientY: 120 });
    fireEvent.pointerUp(title, { button: 0, pointerId: 2, clientX: 200, clientY: 120 });
    fireEvent.click(title, { detail: 1 });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  } finally { rect.mockRestore(); }
});

it("keeps tiny pointer movement as a click and excludes the minimize control from dragging", async () => {
  await openDevices();
  const title = screen.getByRole("button", { name: "Tiến trình công việc" });
  const panel = screen.getByRole("dialog", { name: "Cửa sổ tiến trình" });
  fireEvent.pointerDown(title, { button: 0, isPrimary: true, pointerId: 1, clientX: 100, clientY: 100 });
  fireEvent.pointerMove(title, { pointerId: 1, clientX: 102, clientY: 101 });
  expect(HTMLElement.prototype.setPointerCapture).not.toHaveBeenCalled();
  fireEvent.pointerUp(title, { pointerId: 1 });
  const minimize = screen.getByRole("button", { name: "Thu nhỏ tiến trình" });
  fireEvent.pointerDown(minimize, { button: 0, isPrimary: true, pointerId: 2, clientX: 100, clientY: 100 });
  fireEvent.pointerMove(minimize, { pointerId: 2, clientX: 180, clientY: 150 });
  fireEvent.pointerUp(minimize, { pointerId: 2 });
  fireEvent.keyDown(minimize, { key: "ArrowLeft" });
  expect(panel.style.left).toBe("");
  expect(HTMLElement.prototype.setPointerCapture).not.toHaveBeenCalled();
  fireEvent.click(title, { detail: 1 });
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});

it("cancels a touch drag without blocking subsequent keyboard activation", async () => {
  await openDevices();
  const title = screen.getByRole("button", { name: "Tiến trình công việc" });
  fireEvent.pointerDown(title, { button: 0, isPrimary: true, pointerId: 1, pointerType: "touch", clientX: 100, clientY: 100 });
  fireEvent.pointerMove(title, { pointerId: 1, pointerType: "touch", clientX: 180, clientY: 150 });
  fireEvent.pointerCancel(title, { pointerId: 1, pointerType: "touch" });
  fireEvent.click(title, { detail: 0 });
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  fireEvent.click(title);
  expect(screen.getByRole("dialog", { name: "Cửa sổ tiến trình" })).toBeVisible();
});

async function openDevices(wide = false) {
  render(<OperationProgressCenter deviceLabels={labels} />);
  fireEvent.click(await screen.findByLabelText("Tiến trình công việc"));
  await screen.findByRole("button", { name: /Máy 2/ });
  if (wide) fireEvent.click(screen.getByRole("button", { name: "Phóng rộng tiến trình" }));
}

it("opens run, exact device and HH:mm:ss log without dispatching work", async () => {
  await openDevices();
  expect(screen.getByRole("progressbar", { name: "Tiến độ công việc" })).toHaveAttribute("aria-valuenow", "50");
  expect(operationDeviceLog).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: /Máy 2/ }));
  expect(await screen.findByText("12:34:56")).toBeVisible();
  expect(operationDeviceLog).toHaveBeenLastCalledWith("publish:run", "a");
  fireEvent.keyDown(screen.getByLabelText("Tiến trình công việc"), { key: "Escape" });
  await waitFor(() => expect(screen.getByLabelText("Chi tiết máy")).not.toBeVisible());
});

it.each(["running", "partial"] as const)("does not display complete progress for a %s post awaiting verification", async (state) => {
  const pending = { ...run, state, completedItems: state === "running" ? 1 : 2, retryScope: "linkAndSheet" as const };
  vi.mocked(operationQueryRuns).mockResolvedValue({ runs: [pending], total: 1, counts: { active: 0, succeeded: 0, attention: 1 }, hasMore: false });
  vi.mocked(operationGetRun).mockResolvedValue({ ...detail, summary: pending, items: [detail.items[0], {
    ...detail.items[1], state, errorCode: "post_verification_pending",
  }] });
  await openDevices();
  const progress = screen.getByRole("progressbar", { name: "Tiến độ công việc" });
  if (state === "running") expect(progress).toHaveAttribute("aria-valuenow", "50");
  else expect(progress).not.toHaveAttribute("aria-valuenow");
  expect(screen.queryByText("100%")).toBeNull();
  expect(screen.getByRole("button", { name: /Máy 5.*Chờ xác minh bài đăng/ })).toBeVisible();
  expect(screen.getByLabelText("Kết quả từng máy")).toHaveTextContent("1 hoàn tất");
  expect(screen.getByLabelText("Kết quả từng máy")).toHaveTextContent(state === "running" ? "1 đang chờ/chạy" : "1 cần kiểm tra");
});

it("renders numbered Publish steps with original times in both sort directions", async () => {
  vi.mocked(operationDeviceLog).mockResolvedValue({ entries: [
    { id: "1", at: "2026-09-08T07:01:02", action: "publishStep", state: "opening_app", text: "[3] Đang mở TikTok và chờ màn hình sẵn sàng", detail: null },
    { id: "2", at: "2026-09-08T07:02:03", action: "publishStep", state: "finished", text: "[14] Thành công — máy 2 đã đăng bài", detail: null },
  ], truncated: false });
  await openDevices();
  fireEvent.click(screen.getByRole("button", { name: /Máy 2/ }));
  const list = await screen.findByRole("list", { name: "Nhật ký theo thời gian" });
  expect(within(list).getAllByRole("listitem")[0]).toHaveTextContent("07:02:03[14] Thành công");
  fireEvent.click(screen.getByRole("button", { name: "Mới nhất trước" }));
  expect(within(list).getAllByRole("listitem")[0]).toHaveTextContent("07:01:02[3] Đang mở TikTok");
  expect(operationDeviceLog).toHaveBeenLastCalledWith(run.id, "a");
});

it("keeps the recorded machine number after the current fleet changes", async () => {
  vi.mocked(operationGetRun).mockResolvedValue({ ...detail, items: [{ ...detail.items[0], label: "Máy 8" }] });
  render(<OperationProgressCenter deviceLabels={new Map([["a", "Máy 2 · SM G955F"]])} />);
  fireEvent.click(await screen.findByLabelText("Tiến trình công việc"));
  expect(await screen.findByRole("button", { name: /Máy 8/ })).toBeVisible();
  expect(screen.queryByRole("button", { name: /Máy 2/ })).toBeNull();
});

it("ignores a late log from the previously selected phone", async () => {
  let resolve!: (value: Awaited<ReturnType<typeof operationDeviceLog>>) => void;
  vi.mocked(operationDeviceLog).mockImplementationOnce(() => new Promise((done) => { resolve = done; }));
  await openDevices();
  fireEvent.click(screen.getByRole("button", { name: /Máy 2/ }));
  await waitFor(() => expect(operationDeviceLog).toHaveBeenCalledWith(run.id, "a"));
  fireEvent.click(screen.getByRole("button", { name: "Về danh sách máy" }));
  fireEvent.click(screen.getByRole("button", { name: /Máy 5/ }));
  await screen.findByText("12:34:56");
  resolve({ entries: [{ id: "old", at: null, action: "nurture", state: "", text: "Old phone log", detail: null }], truncated: false });
  await waitFor(() => expect(screen.queryByText("Old phone log")).not.toBeInTheDocument());
  expect(within(screen.getByLabelText("Chi tiết máy")).getByText(labels.get("b")!)).toBeVisible();
});

it("shows an inline retry for logs and does not turn a read error into an empty success", async () => {
  vi.mocked(operationDeviceLog).mockRejectedValueOnce(new Error("read failed"));
  await openDevices();
  fireEvent.click(screen.getByRole("button", { name: /Máy 2/ }));
  expect(await screen.findByRole("alert")).toHaveTextContent("read failed");
  fireEvent.click(screen.getByRole("button", { name: "Thử lại" }));
  expect(await screen.findByText("12:34:56")).toBeVisible();
});

it("minimizes without hiding progress and prevents deleting running records", async () => {
  await openDevices();
  expect(screen.getByRole("dialog", { name: "Cửa sổ tiến trình" })).toHaveClass("is-floating");
  expect(screen.getByRole("button", { name: "Xoá bản ghi Đăng bài" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Xoá các bản ghi đã kết thúc" })).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: "Thu nhỏ tiến trình" }));
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(screen.getByRole("progressbar", { name: "Tiến độ công việc" })).toHaveAttribute("aria-valuenow", "50");
  fireEvent.click(screen.getByRole("button", { name: "Mở rộng tiến trình" }));
  expect(await screen.findByRole("button", { name: /Máy 2/ })).toBeVisible();
});

it("deletes a finished record from the monitor, persists and supports undo", async () => {
  const done = { ...run, state: "partial" as const, updatedAt: "2026-09-07T00:00:00Z" };
  vi.mocked(operationQueryRuns).mockResolvedValue({ runs: [done], total: 1, counts: { active: 0, succeeded: 0, attention: 1 }, hasMore: false });
  await openDevices();
  fireEvent.click(screen.getByRole("button", { name: "Xoá bản ghi Đăng bài" }));
  expect(screen.getByText("Không còn bản ghi trong cửa sổ này.")).toBeVisible();
  expect(screen.getByRole("button", { name: "Hoàn tác xoá" })).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "Hoàn tác xoá" }));
  expect(await screen.findByRole("button", { name: "Xoá bản ghi Đăng bài" })).toBeEnabled();
  fireEvent.click(screen.getByRole("button", { name: "Xoá các bản ghi đã kết thúc" }));
  cleanup();
  render(<OperationProgressCenter deviceLabels={labels} />);
  fireEvent.click(await screen.findByLabelText("Tiến trình công việc"));
  expect(await screen.findByText("Không còn bản ghi trong cửa sổ này.")).toBeVisible();
  expect(screen.queryByRole("button", { name: "Xoá bản ghi Đăng bài" })).not.toBeInTheDocument();
});

it("retains a finished record if saving its dismissal fails", async () => {
  vi.mocked(operationQueryRuns).mockResolvedValue({ runs: [{ ...run, state: "succeeded" }], total: 1, counts: { active: 0, succeeded: 1, attention: 0 }, hasMore: false });
  await openDevices();
  const failedWrite = vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw Error("quota reached"); });
  try {
    fireEvent.click(screen.getByRole("button", { name: "Xoá bản ghi Đăng bài" }));
    expect(screen.getByRole("alert")).toHaveTextContent("quota reached");
    expect(screen.getByRole("button", { name: "Xoá bản ghi Đăng bài" })).toBeVisible();
  } finally { failedWrite.mockRestore(); }
});

it("shows one progress bar, filters devices and separates evidence from the timeline", async () => {
  vi.mocked(operationGetRun).mockResolvedValue({ ...detail, items: [
    { ...detail.items[0], state: "failed", evidence: '{"verified":true}' }, detail.items[1],
  ] });
  await openDevices(true);
  expect(screen.getAllByRole("progressbar")).toHaveLength(1);
  fireEvent.change(screen.getByRole("combobox", { name: "Lọc trạng thái máy" }), { target: { value: "issues" } });
  expect(screen.queryByRole("button", { name: /Máy 5/ })).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: /Máy 2/ }));
  await screen.findByText("12:34:56");
  expect(screen.queryByText('{"verified":true}')).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("tab", { name: "Bằng chứng" }));
  expect(screen.getByText('{"verified":true}')).toBeInTheDocument();
  expect(screen.getByLabelText("Nhật ký theo thời gian")).not.toBeVisible();
  fireEvent.change(screen.getByRole("searchbox", { name: "Tìm máy trong tác vụ" }), { target: { value: "missing" } });
  expect(screen.getByText("Không có máy phù hợp.")).toBeVisible();
});

it("keeps the selected device, timeline order and scroll through minimize", async () => {
  await openDevices();
  fireEvent.click(screen.getByRole("button", { name: /Máy 5/ }));
  await screen.findByText("12:34:56");
  fireEvent.click(screen.getByRole("button", { name: "Mới nhất trước" }));
  const timeline = screen.getByLabelText("Nhật ký theo thời gian");
  const scroller = timeline.parentElement!;
  scroller.scrollTop = 120;
  fireEvent.click(screen.getByRole("button", { name: "Thu nhỏ tiến trình" }));
  expect(timeline).not.toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "Mở rộng tiến trình" }));
  expect(within(screen.getByLabelText("Chi tiết máy")).getByText(labels.get("b")!)).toBeVisible();
  expect(screen.getByRole("button", { name: "Cũ nhất trước" })).toBeVisible();
  expect(screen.getByLabelText("Nhật ký theo thời gian")).toBe(timeline);
  expect(scroller.scrollTop).toBe(120);
});

it("filters out stale selection instead of showing details of an invisible device", async () => {
  await openDevices(true);
  fireEvent.click(screen.getByRole("button", { name: /Máy 2/ }));
  await screen.findByText("12:34:56");
  fireEvent.change(screen.getByRole("combobox", { name: "Lọc trạng thái máy" }), { target: { value: "issues" } });
  expect(screen.getByText("Không có máy phù hợp.")).toBeVisible();
  expect(screen.queryByText("12:34:56")).not.toBeInTheDocument();
  expect(screen.getByText("Chọn máy để xem nhật ký")).toBeVisible();
});

it("uses a full-width device detail with Back on narrow viewports", async () => {
  narrowViewport = true;
  await openDevices();
  expect(screen.getByLabelText("Chi tiết máy")).not.toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: /Máy 5/ }));
  await screen.findByText("12:34:56");
  expect(screen.getByRole("button", { name: "Về danh sách máy" })).toBeVisible();
  expect(screen.getByRole("button", { name: "Về danh sách máy" })).toHaveFocus();
  expect(screen.getByLabelText("Tiến độ từng máy")).not.toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "Về danh sách máy" }));
  expect(screen.getByLabelText("Tiến độ từng máy")).toBeVisible();
  expect(screen.getByLabelText("Chi tiết máy")).not.toBeVisible();
});

it("maximizes without losing selection and Escape closes options before minimizing", async () => {
  await openDevices();
  fireEvent.click(screen.getByRole("button", { name: /Máy 5/ }));
  await screen.findByText("12:34:56");
  fireEvent.click(screen.getByRole("button", { name: "Phóng rộng tiến trình" }));
  expect(screen.getByRole("dialog")).toHaveClass("is-maximized");
  expect(screen.getByRole("button", { name: /Máy 5/ })).toHaveAttribute("aria-pressed", "true");
  fireEvent.click(screen.getByRole("button", { name: "Khôi phục kích thước" }));
  const menu = screen.getByLabelText("Tuỳ chọn bản ghi").parentElement!;
  fireEvent.click(screen.getByLabelText("Tuỳ chọn bản ghi"));
  expect(menu).toHaveAttribute("open");
  fireEvent.keyDown(screen.getByLabelText("Tuỳ chọn bản ghi"), { key: "Escape" });
  expect(menu).not.toHaveAttribute("open");
  expect(screen.getByRole("dialog")).toBeVisible();
});

it("does not steal the inspector when a newer run enters the polled list", async () => {
  const newer = { ...run, id: "publish:new", sourceId: "new", title: "Phiên mới" };
  await openDevices(true);
  fireEvent.click(screen.getByRole("button", { name: /Máy 5/ }));
  await screen.findByText("12:34:56");
  vi.mocked(operationQueryRuns).mockResolvedValue({ runs: [newer, run], total: 2, counts: { active: 2, succeeded: 0, attention: 0 }, hasMore: false });
  await screen.findByRole("option", { name: /Phiên mới/ });
  expect(screen.getByRole("combobox", { name: "Chọn tác vụ theo dõi" })).toHaveValue(run.id);
  expect(screen.getByRole("button", { name: /Máy 5/ })).toHaveAttribute("aria-pressed", "true");
});

it("switches the exact run and drops device selection without cross-run log leakage", async () => {
  const second = { ...run, id: "publish:second", sourceId: "second", title: "Đăng bài lần hai" };
  vi.mocked(operationQueryRuns).mockResolvedValue({ runs: [run, second], total: 2, counts: { active: 2, succeeded: 0, attention: 0 }, hasMore: false });
  vi.mocked(operationGetRun).mockImplementation(async (id) => ({ ...detail, summary: id === second.id ? second : run }));
  await openDevices();
  fireEvent.click(screen.getByRole("button", { name: /Máy 2/ }));
  await screen.findByText("12:34:56");
  fireEvent.change(screen.getByRole("combobox", { name: "Chọn tác vụ theo dõi" }), { target: { value: second.id } });
  await screen.findByRole("button", { name: /Máy 2/ });
  expect(screen.queryByText("12:34:56")).not.toBeInTheDocument();
  expect(screen.getByLabelText("Chi tiết máy")).not.toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: /Máy 5/ }));
  await waitFor(() => expect(operationDeviceLog).toHaveBeenLastCalledWith(second.id, "b"));
});

it("starts compact on desktop, opens only the selected log, and expands on demand", async () => {
  await openDevices();
  const monitor = screen.getByRole("dialog");
  expect(monitor).toHaveClass("is-compact");
  expect(screen.getByLabelText("Tiến độ từng máy")).toBeVisible();
  expect(screen.getByLabelText("Chi tiết máy")).not.toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: /Máy 5/ }));
  await screen.findByText("12:34:56");
  expect(screen.getByLabelText("Tiến độ từng máy")).not.toBeVisible();
  expect(screen.getByRole("button", { name: "Về danh sách máy" })).toHaveFocus();
  fireEvent.click(screen.getByRole("button", { name: "Phóng rộng tiến trình" }));
  expect(screen.getByLabelText("Tiến độ từng máy")).toBeVisible();
  expect(screen.getByLabelText("Chi tiết máy")).toBeVisible();
  expect(screen.getByRole("button", { name: /Máy 5/ })).toHaveAttribute("aria-pressed", "true");
  fireEvent.click(screen.getByRole("button", { name: "Khôi phục kích thước" }));
  expect(screen.getByLabelText("Tiến độ từng máy")).not.toBeVisible();
  expect(screen.getByText("12:34:56")).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "Về danh sách máy" }));
  expect(screen.getByLabelText("Tiến độ từng máy")).toBeVisible();
});
