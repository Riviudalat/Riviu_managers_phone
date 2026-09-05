import { StrictMode } from "react";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { DataPage } from "./DataPage";
import type { AnalyticsSummary } from "../types";

const loadSummary = vi.hoisted(() => vi.fn());

const queryOperations = vi.hoisted(() => vi.fn());
vi.mock("../api", () => ({ analyticsSummary: loadSummary, operationQueryRuns: queryOperations }));
vi.mock("../components/OperationLog", () => ({
  OperationLog: () => <div data-testid="operation-log" />,
}));

const summary: AnalyticsSummary = {
  deviceTotal: 3,
  deviceReady: 2,
  jobsTotal: 7,
  jobsSucceeded: 5,
  jobsFailed: 1,
  jobsRunning: 1,
  scriptsTotal: 4,
  materialsTotal: 6,
  appsTotal: 2,
  schedulesEnabled: 3,
  recentLogs: [],
};

beforeEach(() => {
  loadSummary.mockReset();
  queryOperations.mockReset();
  queryOperations.mockResolvedValue({ runs:[],total:0,counts:{active:0,succeeded:0,attention:0},hasMore:false });
});

describe("DataPage load states", () => {
  it("uses all operation counts instead of the legacy JSON job counters", async () => {
    loadSummary.mockResolvedValue({ ...summary,jobsFailed:0 });
    queryOperations.mockResolvedValue({ runs:[],total:517,counts:{active:3,succeeded:509,attention:5},hasMore:true });
    render(<DataPage />);
    expect(await screen.findByText("5 tác vụ cần xử lý")).toBeVisible();
    expect(screen.getByText("517")).toBeVisible();
    expect(queryOperations).toHaveBeenCalledWith(expect.objectContaining({since:expect.any(String),limit:1}));
  });
  it("shows loading before the summary, then data without repeating the topbar title", async () => {
    loadSummary.mockResolvedValue(summary);

    render(<DataPage />);

    expect(screen.getByRole("status")).toHaveTextContent("Đang tải dữ liệu");
    expect(screen.queryByRole("heading", { level: 2 })).toBeNull();
    expect(await screen.findByText("2/3")).toBeInTheDocument();
    expect(screen.getByText("Thiết bị")).toBeInTheDocument();
  });

  it("keeps a failed load on the page and retries it in place", async () => {
    loadSummary
      .mockRejectedValueOnce(new Error("database is locked"))
      .mockResolvedValueOnce(summary);

    render(<DataPage />);

    expect(await screen.findByRole("alert")).toHaveTextContent("database is locked");
    expect(screen.queryByTestId("operation-log")).toBeNull();

    await userEvent.click(screen.getByRole("button", { name: "Thử lại" }));

    await waitFor(() => expect(loadSummary).toHaveBeenCalledTimes(2));
    expect(await screen.findByText("2/3")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("keeps the newest summary when an older StrictMode request answers late", async () => {
    let resolveFirst!: (value: AnalyticsSummary) => void;
    let resolveSecond!: (value: AnalyticsSummary) => void;
    loadSummary
      .mockReturnValueOnce(new Promise((resolve) => { resolveFirst = resolve; }))
      .mockReturnValueOnce(new Promise((resolve) => { resolveSecond = resolve; }));

    render(<StrictMode><DataPage /></StrictMode>);
    await waitFor(() => expect(loadSummary).toHaveBeenCalledTimes(2));

    resolveSecond({ ...summary, deviceReady: 4, deviceTotal: 5 });
    expect(await screen.findByText("4/5")).toBeInTheDocument();
    resolveFirst({ ...summary, deviceReady: 1, deviceTotal: 9 });
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(screen.getByText("4/5")).toBeInTheDocument();
    expect(screen.queryByText("1/9")).toBeNull();
  });
});
