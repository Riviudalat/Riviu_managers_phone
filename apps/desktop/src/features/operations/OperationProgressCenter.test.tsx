import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { operationDeviceLog, operationGetRun, operationQueryRuns, operationStop, operationStopStatus, publishCancelUnacceptedStart, publishCheckLinks, publishGet, publishResumeVerification, publishRetryAssignment, publishRetrySheetAssignment, publishStartStatus } from "../../api";
import { observePendingPublishStart } from "./publishStartBridge";
import { readQueryClient } from "../../readQuery";
import type { OperationRunDetail, OperationRunSummary } from "../../types";
import { OperationProgressCenter } from "./OperationProgressCenter";
let narrowViewport = false;
vi.mock("../../useMediaQuery", () => ({ useMediaQuery: () => narrowViewport }));

vi.mock("../../api", () => ({ publishStartStatus:vi.fn(async()=>null),publishCancelUnacceptedStart:vi.fn(),listDevices:vi.fn(async()=>[]),publishGet:vi.fn(async()=>({assignments:[]})),publishRecoveryCapabilities:vi.fn(async()=>[]),publishRetryAssignment:vi.fn(),publishRetrySheetAssignment:vi.fn(),publishCheckLinks:vi.fn(),publishResumeVerification:vi.fn(),operationQueryRuns: vi.fn(), operationGetRun: vi.fn(), operationDeviceLog: vi.fn(), nurtureSessionStatus: vi.fn(async () => []),operationStop:vi.fn(),operationStopStatus:vi.fn(async()=>null) }));
const run: OperationRunSummary = { id: "publish:run", sourceId: "run", kind: "publish", title: "Đăng bài", state: "running", targetCount: 2, totalItems: 2, completedItems: 1, issueCount: 0, retryableCount: 0, retryScope: null, createdAt: null, updatedAt: null };
const detail: OperationRunDetail = { summary: run, items: ["a", "b"].map((udid, i) => ({ id: udid, udid, label: `Bài ${i + 1}`, kind: "assignment", state: i ? "running" : "succeeded", detail: null, errorCode: null, evidence: null, retryable: false })) };
const labels = new Map([["a", "Máy 2 · Nội dung"], ["b", "Máy 5 · Đăng bài"]]);
afterEach(cleanup);
// jsdom gives a `hidden` element no accessible name, so the drawer is found by its role alone.
const drawer = () => screen.getAllByRole("complementary", { hidden: true }).find(element => element.getAttribute("aria-label") === "Hàng đợi Cần xử lý")!;
const chip = () => screen.getByRole("button", { name: /^Hàng đợi Cần xử lý/ });
beforeEach(() => {
  readQueryClient.clear();
  narrowViewport = false;
  localStorage.clear();
  vi.clearAllMocks();
  vi.mocked(operationStopStatus).mockResolvedValue(null);
  HTMLElement.prototype.setPointerCapture = vi.fn();
  vi.mocked(operationQueryRuns).mockResolvedValue({ runs: [run], total: 1, counts: { active: 1, succeeded: 0, attention: 0 }, hasMore: false });
  vi.mocked(operationGetRun).mockResolvedValue(detail);
  vi.mocked(publishGet).mockResolvedValue({ assignments: [] } as never);
  vi.mocked(operationDeviceLog).mockResolvedValue({ entries: [{ id: "log", at: "2026-09-07T12:34:56", action: "publish", state: "posting", text: null, detail: null }], truncated: false });
});

it("shows a new pending start in the pill without opening and keeps a status error visible before ACK", async () => {
  // The start status is re-read every second, and the next read (no record yet) rightly
  // replaces this error with its own; a loaded run reached the assertion after that poll.
  vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
  try {
    vi.mocked(publishStartStatus).mockRejectedValueOnce(new Error("Chưa đọc được trạng thái lượt đăng"));
    render(<OperationProgressCenter deviceLabels={labels} />);
    await act(async () => observePendingPublishStart({ requestId: "new-start", inputKey: "{}", requestedAt: new Date().toISOString() }));
    // Never auto-open (PLAN §5): the start only shows in the bar until the operator opens it.
    expect(drawer()).not.toBeVisible();
    const pill = screen.getByRole("button", { name: "Tiến trình công việc" });
    await waitFor(() => expect(pill).toHaveAttribute("title", expect.stringContaining("Lượt đăng đang chuẩn bị")));
    fireEvent.click(pill);
    expect(drawer()).toBeVisible();
    expect(await screen.findByText("Chưa đọc được trạng thái lượt đăng")).toBeVisible();
    expect(screen.getByText("Tác vụ: publish-start:new-start")).toBeVisible();
  } finally { vi.useRealTimers(); }
});

it("stops only the selected operation and waits for a backend cleanup result",async()=>{
 await openDevices();
 expect(screen.getByRole("button",{name:"Dừng tác vụ và về màn hình chính"})).toHaveTextContent("Tạm dừng");
 vi.mocked(operationStop).mockResolvedValue({operationId:run.id,state:"stopping",devices:[{udid:"a",closed:false,message:"waiting"}]});
 fireEvent.click(screen.getByRole("button",{name:"Dừng tác vụ và về màn hình chính"}));
 await waitFor(()=>expect(operationStop).toHaveBeenCalledWith(run.id));
 expect((await screen.findAllByText("Đang dừng và nhả máy…")).length).toBeGreaterThan(0);
 expect(screen.getByRole("button",{name:"Dừng tác vụ và về màn hình chính"})).toBeDisabled();
 expect(operationStop).toHaveBeenCalledTimes(1);
});

it("reports a rejected stop without removing the run",async()=>{
 await openDevices();vi.mocked(operationStop).mockRejectedValue(new Error("cannot persist stop"));
 fireEvent.click(screen.getByRole("button",{name:"Dừng tác vụ và về màn hình chính"}));
 expect(await screen.findByText("cannot persist stop")).toBeVisible();
 expect(screen.getByRole("combobox",{name:"Chọn tác vụ theo dõi"})).toHaveValue(run.id);
});

it("keeps an acknowledged stop pending when a later status read returns no record", async () => {
 vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
 try {
  await openDevices();
  vi.mocked(operationStop).mockResolvedValue({operationId:run.id,state:"stopping",devices:[{udid:"a",closed:false,message:"waiting"}]});
  fireEvent.click(screen.getByRole("button",{name:"Dừng tác vụ và về màn hình chính"}));
  await waitFor(()=>expect(operationStop).toHaveBeenCalledWith(run.id));
  await act(async()=>{await vi.advanceTimersByTimeAsync(2100);});
  expect(operationStopStatus).toHaveBeenCalledTimes(2);
  expect(screen.getByRole("button",{name:"Dừng tác vụ và về màn hình chính"})).toBeDisabled();
  expect(operationStop).toHaveBeenCalledTimes(1);
 } finally { vi.useRealTimers(); }
});

it("offers pause for a legacy stopped publication that has not released its devices",async()=>{
 const stopped={...run,state:"cancelled" as const};
 // Older than the 24 h list: only the all-age reads find it, and Theo dõi still opens exactly it.
 vi.mocked(operationQueryRuns).mockImplementation(async query => query.since
  ? {runs:[],total:0,counts:{active:0,succeeded:0,attention:0},hasMore:false}
  : {runs:[stopped],total:1,counts:{active:0,succeeded:0,attention:1},hasMore:false});
 vi.mocked(operationStopStatus).mockResolvedValue({operationId:run.id,state:"closed",devices:[]});
 // A finished run leaves "Đang chạy"; its Chưa rõ post keeps it reachable from the queue.
 vi.mocked(publishGet).mockResolvedValue({ campaign: { createdAt: "2026-09-07T12:00:00Z" }, assignments: [{ id: "a", udid: "a", ordinal: 0, state: "uncertain", effectIntent: "post" }] } as never);
 await openDevices(false, stopped);
 expect(screen.getByRole("combobox",{name:"Chọn tác vụ theo dõi"})).toHaveValue(run.id);
 expect(screen.getByRole("button",{name:"Dừng tác vụ và về màn hình chính"})).toHaveTextContent("Tạm dừng");
});

it("opens over the wall from the bar, never docks, keeps focus where it was and closes back to its launcher", async () => {
  await openDevices();
  const pill = screen.getByRole("button", { name: "Tiến trình công việc" });
  expect(pill).toHaveAttribute("aria-expanded", "true");
  // An overlay on document.body, not a column beside the wall: nothing it does takes width.
  expect(drawer().parentElement).toBe(document.body);
  expect(drawer()).not.toHaveClass("is-docked");
  fireEvent.click(pill);
  expect(drawer()).not.toBeVisible();
  pill.focus();
  fireEvent.click(pill);
  expect(drawer()).toBeVisible();
  expect(pill).toHaveFocus();
  screen.getByRole("button", { name: "Đóng hàng đợi" }).focus();
  fireEvent.keyDown(screen.getByRole("button", { name: "Đóng hàng đợi" }), { key: "Escape" });
  expect(drawer()).not.toBeVisible();
  expect(pill).toHaveFocus();
});

it("counts a new item on the chip with a notice and never opens the queue on its own", async () => {
  vi.mocked(publishGet).mockResolvedValue({ campaign: { createdAt: "2026-09-07T12:00:00Z" }, assignments: [] } as never);
  render(<OperationProgressCenter deviceLabels={labels} />);
  await waitFor(() => expect(chip()).toHaveAccessibleName("Hàng đợi Cần xử lý: 0 mục"));
  vi.mocked(publishGet).mockResolvedValue({ campaign: { createdAt: "2026-09-07T12:00:00Z" }, assignments: [{ id: "a", udid: "a", ordinal: 0, state: "uncertain", effectIntent: "post" }] } as never);
  await act(async () => { await readQueryClient.invalidateQueries(); });
  await waitFor(() => expect(chip()).toHaveAccessibleName("Hàng đợi Cần xử lý: 1 mục"));
  expect(screen.getByText(/Cần xử lý: thêm 1 mục\. Máy 2 · Chưa rõ bài đã lên/)).toBeInTheDocument();
  expect(drawer()).not.toBeVisible();
  expect(chip()).toHaveAttribute("aria-expanded", "false");
  // A source that stopped at its page size is never read as complete: the count becomes a floor.
  vi.mocked(operationQueryRuns).mockResolvedValue({ runs: [run], total: 201, counts: { active: 1, succeeded: 0, attention: 0 }, hasMore: true });
  await act(async () => { await readQueryClient.invalidateQueries(); });
  await waitFor(() => expect(chip()).toHaveAccessibleName("Hàng đợi Cần xử lý: ít nhất 1 mục; một số nguồn chưa đọc được"));
  expect(chip()).toHaveTextContent("≥1");
});

it("toggles with Ctrl+Shift+X without moving focus, and the Chưa rõ row offers only evidence and Theo dõi", async () => {
  vi.mocked(publishGet).mockResolvedValue({ campaign: { createdAt: "2026-09-07T12:00:00Z" }, assignments: [{ id: "a", udid: "a", ordinal: 0, state: "uncertain", effectIntent: "post" }] } as never);
  render(<><button type="button">Máy 9 trên lưới</button><OperationProgressCenter deviceLabels={labels} /></>);
  const tile = screen.getByRole("button", { name: "Máy 9 trên lưới" });
  tile.focus();
  fireEvent.keyDown(window, { key: "X", ctrlKey: true, shiftKey: true });
  expect(drawer()).toBeVisible();
  expect(tile).toHaveFocus();
  const row = await screen.findByRole("listitem", { name: /^Máy 2 · Chưa thuộc box · Chưa rõ bài đã lên/ });
  expect(within(row).getByText("Ở lại tới khi có kết luận")).toBeVisible();
  expect(within(row).getAllByRole("button").map(button => button.textContent)).toEqual(["Theo dõi"]);
  expect(within(row).getByText("Xem bằng chứng")).toBeVisible();
  // A failed refresh is not a conclusion: the row stays, beside the read error.
  vi.mocked(publishGet).mockRejectedValue(new Error("đọc chiến dịch thất bại"));
  await act(async () => { await readQueryClient.invalidateQueries(); });
  expect(await screen.findByText(/Chưa đọc lại được lượt này: đọc chiến dịch thất bại/)).toBeVisible();
  expect(screen.getByRole("listitem", { name: /^Máy 2 · Chưa thuộc box · Chưa rõ bài đã lên/ })).toBeVisible();
  expect(chip()).toHaveAccessibleName("Hàng đợi Cần xử lý: ít nhất 1 mục; một số nguồn chưa đọc được");
  fireEvent.keyDown(window, { key: "X", ctrlKey: true, shiftKey: true });
  expect(drawer()).not.toBeVisible();
  expect(tile).toHaveFocus();
});

/** Opens "Đang chạy" on `target`: from the pill while it runs, from its queue row once finished. */
async function openDevices(wide = false, target: OperationRunSummary = run) {
  render(<OperationProgressCenter deviceLabels={labels} />);
  if (target.state === "running" || target.state === "queued") fireEvent.click(await screen.findByLabelText("Tiến trình công việc"));
  else {
    fireEvent.click(chip());
    fireEvent.click((await screen.findAllByRole("button", { name: "Theo dõi" }))[0]);
  }
  await screen.findByRole("button", { name: /Máy 2/ });
  if (wide) fireEvent.click(screen.getByRole("button", { name: "Phóng rộng tiến trình" }));
}

it("opens run, exact device and HH:mm:ss log without dispatching work", async () => {
  await openDevices();
  expect(screen.getByRole("progressbar", { name: "Tiến độ công việc" })).toHaveAttribute("aria-valuenow", "50");
  // Publish rows read their own device timeline for per-device progress. Opening the monitor
  // may read this run's devices, but it never dispatches a stop, retry, link check or Sheet write.
  expect(vi.mocked(operationDeviceLog).mock.calls.every(([operationId, udid]) => operationId === run.id && ["a", "b"].includes(udid))).toBe(true);
  for (const effect of [operationStop, publishCancelUnacceptedStart, publishRetryAssignment, publishRetrySheetAssignment, publishCheckLinks, publishResumeVerification]) {
    expect(effect).not.toHaveBeenCalled();
  }
  fireEvent.click(screen.getByRole("button", { name: /Máy 2/ }));
  expect(await screen.findByText("12:34:56")).toBeVisible();
  expect(operationDeviceLog).toHaveBeenCalledWith("publish:run", "a");
  expect(within(screen.getByLabelText("Chi tiết máy")).getByText(labels.get("a")!)).toBeVisible();
  fireEvent.keyDown(screen.getByRole("button", { name: "Đóng hàng đợi" }), { key: "Escape" });
  await waitFor(() => expect(screen.getByLabelText("Chi tiết máy")).not.toBeVisible());
});

it.each(["running", "partial"] as const)("does not display complete progress for a %s post awaiting verification", async (state) => {
  const pending = { ...run, state, completedItems: state === "running" ? 1 : 2, retryScope: "linkAndSheet" as const };
  vi.mocked(operationQueryRuns).mockResolvedValue({ runs: [pending], total: 1, counts: { active: 0, succeeded: 0, attention: 1 }, hasMore: false });
  vi.mocked(operationGetRun).mockResolvedValue({ ...detail, summary: pending, items: [detail.items[0], {
    ...detail.items[1], state, errorCode: "post_verification_pending",
  }] });
  // The campaign behind the run: one finished post, one still awaiting proof. In the partial run
  // the finished post's evidence and link name @other while the frozen intent says @fixture.
  const conflict = state === "partial";
  vi.mocked(publishGet).mockResolvedValue({ campaign: { createdAt: "2026-09-07T12:00:00Z" }, assignments: [
    { id: "a", udid: "a", ordinal: 0, state: "succeeded", effectIntent: conflict ? JSON.stringify({ effectIntent: "post", expectedAccount: "fixture" }) : "post",
      evidenceJson: JSON.stringify({ post: { publicationVerified: true, postUrl: `https://www.tiktok.com/@${conflict ? "other" : "fixture"}/video/1`, expectedAccount: conflict ? "other" : "fixture" } }) },
    { id: "b", udid: "b", ordinal: 1, state: "verifying", effectIntent: "post", errorCode: "post_verification_pending" },
  ] } as never);
  await openDevices(false, pending);
  // A post awaiting proof makes the whole run indeterminate, running or not: no percentage.
  expect(screen.getByRole("progressbar", { name: "Tiến độ công việc" })).not.toHaveAttribute("aria-valuenow");
  expect(screen.queryByText("100%")).toBeNull();
  expect(screen.getByRole("button", { name: /Máy 5.*Đang xác minh/ })).toBeVisible();
  // Separate numbers from the same campaign read as the detail page; a submitted post is never done.
  await waitFor(() => expect(screen.getByLabelText("Kết quả từng máy")).toHaveTextContent(conflict
    ? "2 đã gửi · 0 đã xác minh · 0 đã ghi Sheet · 1 chưa rõ · 1 đang xác minh"
    : "2 đã gửi · 1 đã xác minh · 0 đã ghi Sheet · 0 chưa rõ · 1 đang xác minh"));
  if (state === "running") expect(screen.getByLabelText("Kết quả từng máy")).toHaveTextContent("1 đang chờ/chạy");
  // The row badge, the selected device's header and the detail read the same full assignment.
  const verdict = conflict ? "Chưa rõ" : "Đã xác minh";
  expect(screen.getByRole("button", { name: new RegExp(`Máy 2.*${verdict}`) })).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: /Máy 2/ }));
  const detailPane = screen.getByLabelText("Chi tiết máy");
  expect(within(detailPane).getAllByText(verdict, { exact: true }).length).toBeGreaterThanOrEqual(2);
  if (conflict) {
    expect(within(detailPane).queryByText("Đã xác minh", { exact: true })).toBeNull();
    expect(within(detailPane).getByText(/Bằng chứng ghi tài khoản @other, khác tài khoản đã ghi trước khi Đăng @fixture/)).toBeVisible();
  }
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
  // The device row polls the same timeline, so keep the read failing until the operator retries.
  let failing = true;
  const succeed = vi.mocked(operationDeviceLog).getMockImplementation()!;
  vi.mocked(operationDeviceLog).mockImplementation(async (operationId, udid) => {
    if (failing && udid === "a") throw new Error("read failed");
    return succeed(operationId, udid);
  });
  await openDevices();
  fireEvent.click(screen.getByRole("button", { name: /Máy 2/ }));
  expect(await screen.findByRole("alert")).toHaveTextContent("read failed");
  expect(screen.queryByText("Chưa có nhật ký cho máy này.")).toBeNull();
  failing = false;
  fireEvent.click(screen.getByRole("button", { name: "Thử lại" }));
  expect(await screen.findByText("12:34:56")).toBeVisible();
});

it("closes without hiding progress and offers no deletion of running records", async () => {
  await openDevices();
  expect(screen.queryByRole("button", { name: /Xoá bản ghi/ })).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Đóng hàng đợi" }));
  expect(drawer()).not.toBeVisible();
  expect(screen.getByRole("progressbar", { name: "Tiến độ các lượt đang chạy" })).toHaveAttribute("aria-valuenow", "50");
  fireEvent.click(screen.getByRole("button", { name: "Tiến trình công việc" }));
  expect(await screen.findByRole("button", { name: /Máy 2/ })).toBeVisible();
});

const failedNurture: OperationRunSummary = { ...run, id: "nurture:done", sourceId: "done", kind: "nurture", title: "Nuôi TikTok", state: "partial", issueCount: 2, updatedAt: "2026-09-07T00:00:00Z" };
const failedNurtureDetail: OperationRunDetail = { summary: failedNurture, items: [
  { id: "f", udid: "a", label: "Máy 2", kind: "device", state: "failed", detail: "TikTok không lên màn hình chính", errorCode: null, evidence: null, retryable: false },
  { id: "u", udid: "b", label: "Máy 5", kind: "device", state: "uncertain", detail: "Đã bấm Gửi nhưng chưa thấy bình luận", errorCode: null, evidence: null, retryable: false },
] };

it("hides a finished run's failures once seen, persists and supports undo, while Chưa rõ stays", async () => {
  vi.mocked(operationQueryRuns).mockResolvedValue({ runs: [failedNurture], total: 1, counts: { active: 0, succeeded: 0, attention: 1 }, hasMore: false });
  vi.mocked(operationGetRun).mockResolvedValue(failedNurtureDetail);
  render(<OperationProgressCenter deviceLabels={labels} />);
  fireEvent.click(chip());
  expect(await screen.findByRole("listitem", { name: /^Máy 2 · .*Thất bại/ })).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "Đã xem các mục lỗi của Nuôi TikTok" }));
  expect(screen.queryByRole("listitem", { name: /^Máy 2 · / })).toBeNull();
  expect(screen.getByRole("listitem", { name: /^Máy 5 · .*Chưa rõ kết quả thao tác/ })).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "Hoàn tác" }));
  expect(await screen.findByRole("listitem", { name: /^Máy 2 · / })).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "Đã xem các mục lỗi của Nuôi TikTok" }));
  cleanup();
  render(<OperationProgressCenter deviceLabels={labels} />);
  fireEvent.click(chip());
  expect(await screen.findByRole("listitem", { name: /^Máy 5 · / })).toBeVisible();
  expect(screen.queryByRole("listitem", { name: /^Máy 2 · / })).toBeNull();
  expect(screen.queryByRole("button", { name: "Đã xem các mục lỗi của Nuôi TikTok" })).toBeNull();
  // Days later no list returns the run (outside 24 h, beyond every page): the Chưa rõ row stays
  // until a read of the run itself shows a conclusion, and only then leaves.
  cleanup();
  readQueryClient.clear();
  vi.mocked(operationQueryRuns).mockResolvedValue({ runs: [], total: 0, counts: { active: 0, succeeded: 0, attention: 0 }, hasMore: false });
  render(<OperationProgressCenter deviceLabels={labels} />);
  fireEvent.click(chip());
  expect(await screen.findByRole("listitem", { name: /^Máy 5 · .*Chưa rõ kết quả thao tác/ })).toBeVisible();
  vi.mocked(operationGetRun).mockResolvedValue({ ...failedNurtureDetail, items: failedNurtureDetail.items.map(item => ({ ...item, state: "succeeded" as const })) });
  await act(async () => { await readQueryClient.invalidateQueries(); });
  await waitFor(() => expect(screen.queryByRole("listitem", { name: /^Máy 5 · / })).toBeNull());
  await waitFor(() => expect(localStorage.getItem("riviu.queue.unresolved.v1")).toBe("[]"));
});

it("retains a finished run's failures if saving the dismissal fails", async () => {
  vi.mocked(operationQueryRuns).mockResolvedValue({ runs: [failedNurture], total: 1, counts: { active: 0, succeeded: 0, attention: 1 }, hasMore: false });
  vi.mocked(operationGetRun).mockResolvedValue(failedNurtureDetail);
  render(<OperationProgressCenter deviceLabels={labels} />);
  fireEvent.click(chip());
  await screen.findByRole("listitem", { name: /^Máy 2 · / });
  const failedWrite = vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw Error("quota reached"); });
  try {
    fireEvent.click(screen.getByRole("button", { name: "Đã xem các mục lỗi của Nuôi TikTok" }));
    expect(screen.getByRole("alert")).toHaveTextContent("quota reached");
    expect(screen.getByRole("listitem", { name: /^Máy 2 · / })).toBeVisible();
  } finally { failedWrite.mockRestore(); }
});

it("shows one progress bar, filters devices and separates evidence from the timeline", async () => {
  vi.mocked(operationGetRun).mockResolvedValue({ ...detail, items: [
    { ...detail.items[0], state: "failed", evidence: '{"verified":true}' }, detail.items[1],
  ] });
  await openDevices(true);
  expect(within(screen.getByRole("tabpanel", { name: "Đang chạy" })).getAllByRole("progressbar")).toHaveLength(1);
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
  fireEvent.click(screen.getByRole("button", { name: "Đóng hàng đợi" }));
  expect(timeline).not.toBeVisible();
  const pill = screen.getByRole("button", { name: "Tiến trình công việc" });
  pill.focus();
  fireEvent.click(pill);
  // Showing the kept phone detail again is not a new selection: focus stays on the opener.
  expect(pill).toHaveFocus();
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

it("maximizes without losing selection and Escape inside the drawer closes it", async () => {
  await openDevices();
  fireEvent.click(screen.getByRole("button", { name: /Máy 5/ }));
  await screen.findByText("12:34:56");
  fireEvent.click(screen.getByRole("button", { name: "Phóng rộng tiến trình" }));
  expect(drawer()).toHaveClass("is-maximized");
  expect(screen.getByRole("button", { name: /Máy 5/ })).toHaveAttribute("aria-pressed", "true");
  fireEvent.click(screen.getByRole("button", { name: "Khôi phục kích thước" }));
  expect(drawer()).not.toHaveClass("is-maximized");
  fireEvent.keyDown(screen.getByRole("button", { name: "Phóng rộng tiến trình" }), { key: "Escape" });
  expect(drawer()).not.toBeVisible();
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
  const monitor = screen.getByRole("tabpanel", { name: "Đang chạy" });
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
