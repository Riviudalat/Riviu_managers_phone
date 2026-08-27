import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { OperationLog } from "./OperationLog";
import type { OpLog } from "../types";

const listLogs = vi.hoisted(() => vi.fn());

// Every export this component reaches for. An object-literal `vi.mock` returns `undefined` for
// anything it omits, and calling `undefined()` throws during render — which fails every test in
// the file at once with a React stack rather than a message about the mock.
vi.mock("../api", () => ({ listOpLogs: listLogs }));

function row(over: Partial<OpLog>): OpLog {
  return {
    id: "l1",
    action: "nurture.start",
    detail: "10969614 · 18 video",
    createdAt: "2026-08-27T09:14:02.512Z",
    ...over,
  };
}

beforeEach(() => {
  listLogs.mockReset();
});
afterEach(cleanup);

describe("the operation log", () => {
  /**
   * **The table had fifteen writers and no reader.**
   *
   * `log_op` is called from the nurture engine, the publish path, the farm commands and eight
   * places in `state.rs`. `analytics_summary` selected the last twenty rows into `recentLogs`,
   * and `DataPage` rendered eight stat tiles and dropped that field — so the app's own record of
   * what it did was invisible inside the app that wrote it.
   */
  it("renders what the app recorded itself doing", async () => {
    listLogs.mockResolvedValue([
      row({ id: "a", action: "nurture.start", detail: "10969614 · 18 video" }),
      row({ id: "b", action: "publish.create", detail: "quán ăn 3" }),
    ]);

    render(<OperationLog />);

    await waitFor(() => expect(screen.getByText("nurture.start")).toBeInTheDocument());
    expect(screen.getByText("publish.create")).toBeInTheDocument();
    expect(screen.getByText(/10969614 · 18 video/)).toBeInTheDocument();
  });

  /** Asks for more than the twenty `analytics_summary` bundles, which is the whole reason. */
  it("asks for a window deeper than the analytics summary's twenty", async () => {
    listLogs.mockResolvedValue([]);
    render(<OperationLog />);
    await waitFor(() => expect(listLogs).toHaveBeenCalled());
    const [limit] = listLogs.mock.calls[0];
    expect(limit).toBeGreaterThan(20);
  });

  /**
   * The filter matches detail as well as action.
   *
   * A udid is in the detail, never in the action, and a udid is what an operator has when they
   * are asking what happened to one phone.
   */
  it("filters on the detail, not just the action", async () => {
    listLogs.mockResolvedValue([
      row({ id: "a", action: "nurture.start", detail: "10969614" }),
      row({ id: "b", action: "nurture.start", detail: "23021RAAEG" }),
    ]);
    render(<OperationLog />);
    await waitFor(() => expect(screen.getByText("10969614")).toBeInTheDocument());

    await userEvent.type(screen.getByLabelText("Lọc nhật ký thao tác"), "23021");

    expect(screen.queryByText("10969614")).toBeNull();
    expect(screen.getByText("23021RAAEG")).toBeInTheDocument();
  });

  /**
   * **"Nothing matched your filter" and "the table is empty" are different facts.**
   *
   * Showing the empty-table sentence to somebody who has just typed a filter tells them the app
   * has never done anything, which is both wrong and alarming.
   */
  it("tells an empty filter result apart from an empty table", async () => {
    listLogs.mockResolvedValue([row({})]);
    render(<OperationLog />);
    await waitFor(() => expect(screen.getByText("nurture.start")).toBeInTheDocument());

    await userEvent.type(screen.getByLabelText("Lọc nhật ký thao tác"), "khong-co-gi");

    expect(screen.getByText(/Không dòng nào khớp/)).toBeInTheDocument();
    expect(screen.queryByText(/Chưa có dòng nào/)).toBeNull();
  });

  it("says the table is empty when it is", async () => {
    listLogs.mockResolvedValue([]);
    render(<OperationLog />);
    await waitFor(() => expect(screen.getByText(/Chưa có dòng nào/)).toBeInTheDocument());
  });

  /** A read failure is reported, not swallowed into an empty list. */
  it("reports a failed read rather than looking empty", async () => {
    listLogs.mockRejectedValue({ code: "OperationFailed", message: "database is locked" });
    render(<OperationLog />);

    await waitFor(() => expect(screen.getByRole("alert")).toBeInTheDocument());
    expect(screen.getByRole("alert").textContent).toContain("database is locked");
    expect(screen.queryByText(/Chưa có dòng nào/)).toBeNull();
  });
});
