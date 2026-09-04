import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AppEvent, OperationRunDetail, OperationRunSummary } from "../types";
import { JobsPanel } from "./JobsPanel";

const operationListRuns = vi.hoisted(() => vi.fn());
const operationGetRun = vi.hoisted(() => vi.fn());
const eventListeners = vi.hoisted(() => [] as Array<(event: AppEvent) => void>);

vi.mock("../api", () => ({
  cancelJob: vi.fn(async () => undefined),
  listenRiviuEvents: vi.fn(async (listener: (event: AppEvent) => void) => {
    eventListeners.push(listener);
    return () => {
      const index = eventListeners.indexOf(listener);
      if (index >= 0) eventListeners.splice(index, 1);
    };
  }),
  operationGetRun,
  operationListRuns,
  runScript: vi.fn(async () => undefined),
}));

const summary: OperationRunSummary = {
  id: "interaction:campaign-a",
  sourceId: "campaign-a",
  kind: "interaction",
  title: "Tương tác · @creator",
  state: "partial",
  targetCount: 2,
  totalItems: 2,
  completedItems: 2,
  issueCount: 1,
  retryableCount: 1,
  retryScope: null,
  createdAt: null,
  updatedAt: "2026-09-04T12:00:00Z",
};

const detail: OperationRunDetail = {
  summary,
  items: [
    {
      id: "assignment-a",
      kind: "assignment",
      label: "Lượt tương tác 1",
      state: "succeeded",
      udid: "phone-1",
      errorCode: null,
      detail: "Bình luận đã gửi",
      evidence: null,
      retryable: false,
    },
    {
      id: "assignment-b",
      kind: "assignment",
      label: "Lượt tương tác 2",
      state: "failed",
      udid: "phone-2",
      errorCode: "BeforeEffect",
      detail: "Không mở được bài",
      evidence: null,
      retryable: true,
    },
  ],
};

function renderPanel() {
  return render(
    <JobsPanel
      devices={[]}
      selectedUdids={[]}
      onSelectUdids={() => undefined}
      deviceLabels={new Map([["phone-1", "Máy 19 · Kệ trên"]])}
    />,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  operationListRuns.mockResolvedValue([]);
  operationGetRun.mockResolvedValue(null);
  eventListeners.splice(0);
});

describe("JobsPanel operations monitor", () => {
  it("leaves the page title to the topbar and renders loading then empty", async () => {
    let resolve!: (runs: OperationRunSummary[]) => void;
    operationListRuns.mockReturnValueOnce(new Promise((done) => { resolve = done; }));

    renderPanel();

    expect(screen.queryByRole("heading", { level: 2 })).toBeNull();
    expect(screen.getByRole("status")).toHaveTextContent("Đang tải tác vụ");
    expect(screen.queryByText("Chưa có tác vụ")).toBeNull();
    resolve([]);
    expect(await screen.findByText("Chưa có tác vụ")).toBeVisible();
  });

  it("keeps a list failure inline and retries only the list", async () => {
    operationListRuns
      .mockRejectedValueOnce(new Error("database is locked"))
      .mockResolvedValueOnce([]);

    renderPanel();

    expect(await screen.findByRole("alert")).toHaveTextContent("database is locked");
    await userEvent.click(screen.getByRole("button", { name: "Thử lại" }));
    await waitFor(() => expect(operationListRuns).toHaveBeenCalledTimes(2));
    expect(await screen.findByText("Chưa có tác vụ")).toBeVisible();
  });

  it("renders normalized data and retries a failed detail independently", async () => {
    operationListRuns.mockResolvedValue([summary]);
    operationGetRun
      .mockRejectedValueOnce(new Error("detail read failed"))
      .mockResolvedValueOnce(detail);

    renderPanel();

    expect(await screen.findByText("Tương tác · @creator")).toBeVisible();
    const runRow = screen.getByRole("button", { name: /Tương tác · @creator/ });
    expect(within(runRow).getByText("Một phần")).toBeVisible();
    expect(await screen.findByRole("alert")).toHaveTextContent("detail read failed");
    await userEvent.click(screen.getByRole("button", { name: "Thử lại chi tiết" }));

    expect(await screen.findByText("Không mở được bài")).toBeInTheDocument();
    expect(screen.getByText("Có thể chạy lại từ nguồn gốc", { exact: false })).toBeVisible();
    expect(operationListRuns).toHaveBeenCalledTimes(1);
    expect(operationGetRun).toHaveBeenCalledTimes(2);
  });

  it("keeps the advanced runner empty and exposes no production fixture", async () => {
    renderPanel();
    expect(await screen.findByText("Chưa có tác vụ")).toBeVisible();

    const advanced = screen.getByText("Chạy JSON nâng cao").closest("details");
    expect(advanced).not.toHaveAttribute("open");

    await userEvent.click(screen.getByText("Chạy JSON nâng cao"));
    expect(advanced).toHaveAttribute("open");
    expect(screen.getByPlaceholderText("Dán kịch bản JSON đã kiểm tra")).toHaveValue("");
    expect(screen.queryByRole("button", { name: "Nạp ví dụ" })).toBeNull();
  });

  it("renders an uncertain publish retry boundary from the typed projection", async () => {
    const publishSummary: OperationRunSummary = {
      ...summary,
      id: "publish:campaign-orphan",
      sourceId: "campaign-orphan",
      kind: "publish",
      title: "Đăng bài",
      state: "uncertain",
      targetCount: 1,
      totalItems: 1,
      completedItems: 1,
      retryableCount: 0,
      retryScope: "none",
    };
    operationListRuns.mockResolvedValue([publishSummary]);
    operationGetRun.mockResolvedValue({ summary: publishSummary, items: [] });

    renderPanel();
    expect(await screen.findByRole("button", { name: /Đăng bài/ })).toBeVisible();
    await screen.findByText("Mã tác vụ và tiến độ");
    await userEvent.click(screen.getByText("Mã tác vụ và tiến độ"));

    expect(screen.getByText("Phạm vi khôi phục: Không tự động chạy lại")).toBeVisible();
    expect(screen.queryByText("Có thể chạy lại từ nguồn gốc", { exact: false })).toBeNull();
  });

  it("keeps the reviewed snapshot label when a publish target is offline", async () => {
    const publishSummary: OperationRunSummary = {
      ...summary,
      id: "publish:campaign-offline",
      sourceId: "campaign-offline",
      kind: "publish",
      title: "Đăng bài",
    };
    operationListRuns.mockResolvedValue([publishSummary]);
    operationGetRun.mockResolvedValue({
      summary: publishSummary,
      items: [{
        ...detail.items[0],
        id: "publish-assignment",
        label: "Máy 13 · Kệ dưới",
        udid: "offline-phone",
      }],
    });

    renderPanel();

    expect(await screen.findByText("Máy 13 · Kệ dưới")).toBeVisible();
    expect(screen.queryByText("Máy trong snapshot")).toBeNull();
  });

  it("refreshes the selected operation from orchestration events and uses the fleet label", async () => {
    const orchestration = {
      ...summary,
      id: "orchestration:run-a",
      sourceId: "run-a",
      kind: "orchestration" as const,
      title: "Điều phối tuần",
    };
    const updated = { ...orchestration, state: "succeeded" as const, issueCount: 0 };
    operationListRuns.mockResolvedValueOnce([orchestration]).mockResolvedValueOnce([updated]);
    operationGetRun
      .mockResolvedValueOnce({ ...detail, summary: orchestration })
      .mockResolvedValueOnce({ ...detail, summary: updated });

    renderPanel();
    expect(await screen.findByText("Máy 19 · Kệ trên")).toBeVisible();
    await waitFor(() => expect(eventListeners).toHaveLength(1));

    eventListeners[0]({ type: "orchestrationUpdated", runId: "run-a" });
    await waitFor(() => expect(operationListRuns).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(operationGetRun).toHaveBeenCalledTimes(2));
    expect(within(screen.getByRole("button", { name: /Điều phối tuần/ })).getByText("Hoàn tất"))
      .toBeVisible();
  });
});
