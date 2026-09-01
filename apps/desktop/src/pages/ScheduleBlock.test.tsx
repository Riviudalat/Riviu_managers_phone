import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import * as api from "../api";
import { ScheduleBlock } from "./ScheduleBlock";

vi.mock("../api", () => ({
  deleteSchedule: vi.fn(async () => undefined),
  exampleScript: vi.fn(async () => "{}"),
  listGroups: vi.fn(async () => []),
  listSchedules: vi.fn(async () => []),
  listScripts: vi.fn(async () => [["daily", "{}"]]),
  saveSchedule: vi.fn(async () => undefined),
  saveScript: vi.fn(async () => undefined),
}));

const mocked = api as unknown as Record<string, ReturnType<typeof vi.fn>>;

beforeEach(() => {
  for (const fn of Object.values(mocked)) fn.mockReset();
  mocked.deleteSchedule.mockResolvedValue(undefined);
  mocked.exampleScript.mockResolvedValue("{}");
  mocked.listGroups.mockResolvedValue([]);
  mocked.listSchedules.mockResolvedValue([]);
  mocked.listScripts.mockResolvedValue([["daily", "{}"]]);
  mocked.saveSchedule.mockResolvedValue(undefined);
  mocked.saveScript.mockResolvedValue(undefined);
});

afterEach(cleanup);

describe("ScheduleBlock states", () => {
  it("shows loading, then a useful empty state", async () => {
    let resolveSchedules: (value: unknown[]) => void = () => undefined;
    mocked.listSchedules.mockImplementationOnce(
      () => new Promise((resolve) => { resolveSchedules = resolve; }),
    );
    render(<ScheduleBlock devices={[]} selected={[]} onSelectUdids={() => {}} />);

    expect(screen.getByText("Đang tải lịch chạy…")).toBeVisible();
    resolveSchedules([]);

    expect(await screen.findByText("Chưa có lịch chạy")).toBeVisible();
    expect(screen.getByRole("group", { name: "Cách lịch chạy" })).not.toHaveAttribute("open");
  });

  it("keeps a load failure in the page and retries to the empty state", async () => {
    mocked.listSchedules
      .mockRejectedValueOnce(new Error("database busy"))
      .mockResolvedValueOnce([]);
    render(<ScheduleBlock devices={[]} selected={[]} onSelectUdids={() => {}} />);

    expect(await screen.findByText(/Không tải được lịch chạy: database busy/)).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: "Thử lại lịch chạy" }));

    expect(await screen.findByText("Chưa có lịch chạy")).toBeVisible();
    expect(mocked.listSchedules).toHaveBeenCalledTimes(2);
  });

  it("maps data state to operator text instead of raw schedule labels", async () => {
    mocked.listSchedules.mockResolvedValue([{
      id: "schedule-1",
      name: "Buổi sáng",
      scriptName: "daily",
      udids: ["A"],
      everyMinutes: 60,
      enabled: true,
      nextRunAt: null,
      lastError: null,
    }]);
    render(<ScheduleBlock devices={[]} selected={[]} onSelectUdids={() => {}} />);

    expect(await screen.findByText("Đang bật")).toBeVisible();
    expect(screen.getByText(/daily · mỗi 60 phút · lần tới chưa lên lịch/)).toBeVisible();
    expect(screen.queryByText(/^on$/)).toBeNull();
    expect(screen.queryByText(/every 60m/)).toBeNull();
  });
});
