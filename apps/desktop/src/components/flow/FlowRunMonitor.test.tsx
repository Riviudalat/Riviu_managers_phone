import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  ActionKind,
  FlowAggregateState,
  FlowAttemptState,
  FlowNodeAttemptRecord,
  FlowRunDetail,
} from "../../types";
import { FlowRunMonitor } from "./FlowRunMonitor";

const api = vi.hoisted(() => ({
  flowGetRun: vi.fn(),
  listenRiviuEvents: vi.fn(),
}));

vi.mock("../../api", () => api);

type EventHandler = (payload: unknown) => void;

function attempt(
  id: string,
  state: FlowAttemptState,
  retryAllowed: boolean,
  actionKind: ActionKind = "tap",
): FlowNodeAttemptRecord {
  return {
    id,
    deviceRunId: "device-run-a",
    nodeId: `node-${id}`,
    actionKind,
    attemptNo: 1,
    sideEffectClass: actionKind === "tap" ? "ambiguousUi" : "none",
    state,
    canonicalInput: { fixture: id },
    evidenceBaseline: null,
    evidenceResult: state === "succeeded" ? { changed: true } : null,
    retryAllowed,
    error: state === "uncertain"
      ? {
          code: "EffectUncertain",
          message: "Effect cannot be proven",
          nodeId: `node-${id}`,
          field: null,
          udid: "device-a",
          attemptId: id,
        }
      : null,
    startedAt: "2026-07-31T01:00:00.000Z",
    updatedAt: "2026-07-31T01:00:00.100Z",
    finishedAt: state === "succeeded" || state === "uncertain"
      ? "2026-07-31T01:00:00.250Z"
      : null,
  };
}

function detail({
  revision = 1,
  state = "running",
  attempts = [attempt("attempt-a", "effectDispatched", false)],
}: {
  revision?: number;
  state?: FlowAggregateState;
  attempts?: FlowNodeAttemptRecord[];
} = {}): FlowRunDetail {
  return {
    run: {
      id: "run-a",
      flowId: "flow-a",
      flowRevision: 3,
      planSha256: "11".repeat(32),
      selection: {
        requested: { mode: "selected", udids: ["device-a"] },
        targetUdids: ["device-a"],
      },
      state,
      eventRevision: revision,
      error: null,
      createdAt: "2026-07-31T01:00:00.000Z",
      updatedAt: "2026-07-31T01:00:00.100Z",
    },
    deviceRuns: [
      {
        id: "device-run-a",
        runId: "run-a",
        udid: "device-a",
        state: state === "succeeded" ? "succeeded" : "running",
        capabilitySnapshot: null,
        releaseProof: null,
        error: null,
        startedAt: "2026-07-31T01:00:00.000Z",
        finishedAt: state === "succeeded" ? "2026-07-31T01:00:00.250Z" : null,
      },
    ],
    attempts,
    artifacts: [],
  };
}

beforeEach(() => {
  api.flowGetRun.mockReset();
  api.listenRiviuEvents.mockReset().mockResolvedValue(vi.fn());
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("FlowRunMonitor retry policy", () => {
  it("uses Vietnamese operator labels and keeps device/node identifiers out of visible text", () => {
    render(
      <FlowRunMonitor
        run={detail({
          state: "succeeded",
          attempts: [attempt("attempt-a", "succeeded", false, "autoSwipe")],
        })}
        devices={[{
          udid: "device-a",
          name: "ONE-01",
          model: "SM-G955U1",
          platform: "android",
          osVersion: "9",
          connection: "usb",
          status: "ready",
          wdaReady: false,
        }]}
        onCancel={vi.fn()}
        onRetry={vi.fn()}
      />,
    );

    expect(screen.getByText("Máy 1 · ONE-01")).toBeInTheDocument();
    expect(screen.getByText("Tự động vuốt")).toBeInTheDocument();
    expect(screen.getAllByText("Thành công")).toHaveLength(2);
    expect(screen.queryByText("device-a")).not.toBeInTheDocument();
    expect(screen.queryByText(/node-attempt/)).not.toBeInTheDocument();
  });

  it("never offers retry for an uncertain attempt and still honors an explicit safe retry", () => {
    const onRetry = vi.fn();
    render(
      <FlowRunMonitor
        run={detail({
          attempts: [
            attempt("uncertain-a", "uncertain", false, "tap"),
            attempt("safe-a", "failedBeforeDispatch", true, "wait"),
          ],
        })}
        onCancel={vi.fn()}
        onRetry={onRetry}
      />,
    );

    const uncertainRow = screen.getByText("Chưa chắc chắn").closest("tr");
    if (uncertainRow === null) throw new Error("uncertain row missing");
    expect(within(uncertainRow).queryByRole("button", { name: /Chạy lại/ })).not.toBeInTheDocument();
    // The cell reads `code: message` now -- the code alone was not enough to tell a timeout from
    // a dead session, because the backend maps several device failures onto one code.
    const technicalError = within(uncertainRow)
      .getByText("Không xác nhận được kết quả thao tác.")
      .closest("details");
    expect(technicalError).toHaveTextContent("EffectUncertain: Effect cannot be proven");

    const safeRetry = screen.getByRole("button", { name: "Chạy lại Chờ" });
    fireEvent.click(safeRetry);
    expect(onRetry).toHaveBeenCalledWith("safe-a");
  });
});

describe("FlowRunMonitor refresh", () => {
  it("treats a matching runtime event as invalidation and fetches the committed projection", async () => {
    let handler: EventHandler | null = null;
    const unlisten = vi.fn();
    api.listenRiviuEvents.mockImplementation(async (next: EventHandler) => {
      handler = next;
      return unlisten;
    });
    api.flowGetRun.mockResolvedValue(
      detail({
        revision: 2,
        state: "succeeded",
        attempts: [attempt("attempt-a", "succeeded", false)],
      }),
    );
    const view = render(
      <FlowRunMonitor run={detail()} onCancel={vi.fn()} onRetry={vi.fn()} />,
    );
    await waitFor(() => expect(api.listenRiviuEvents).toHaveBeenCalledTimes(1));

    await act(async () => {
      handler?.({ type: "flowRunUpdated", runId: "other-run", revision: 99 });
      await Promise.resolve();
    });
    expect(api.flowGetRun).not.toHaveBeenCalled();

    await act(async () => {
      handler?.({ type: "flowRunUpdated", runId: "run-a", revision: 2 });
      await Promise.resolve();
    });
    await waitFor(() => expect(screen.getAllByText("Thành công")).toHaveLength(2));
    expect(api.flowGetRun).toHaveBeenCalledWith("run-a");
    expect(screen.getByText("250 ms")).toBeInTheDocument();

    view.unmount();
    await waitFor(() => expect(unlisten).toHaveBeenCalled());
  });

  it("polls every 750 ms only while nonterminal and stops after the terminal projection", async () => {
    vi.useFakeTimers();
    let handler: EventHandler | null = null;
    api.listenRiviuEvents.mockImplementation(async (next: EventHandler) => {
      handler = next;
      return vi.fn();
    });
    api.flowGetRun.mockResolvedValue(
      detail({
        revision: 2,
        state: "succeeded",
        attempts: [attempt("attempt-a", "succeeded", false)],
      }),
    );
    render(<FlowRunMonitor run={detail()} onCancel={vi.fn()} onRetry={vi.fn()} />);
    await act(async () => {
      await Promise.resolve();
    });
    expect(handler).not.toBeNull();

    act(() => vi.advanceTimersByTime(749));
    expect(api.flowGetRun).not.toHaveBeenCalled();
    await act(async () => {
      vi.advanceTimersByTime(1);
      await Promise.resolve();
    });
    expect(api.flowGetRun).toHaveBeenCalledTimes(1);
    expect(screen.getAllByText("Thành công")).toHaveLength(2);

    await act(async () => {
      vi.advanceTimersByTime(3_000);
      await Promise.resolve();
    });
    expect(api.flowGetRun).toHaveBeenCalledTimes(1);
  });
});

describe("FlowRunMonitor keeps a terminal projection monotonic", () => {
  // A `flowGetRun` started before the run finished can resolve after it, so a projection
  // older than the one on screen arrives through both doors: the parent's prop and this
  // component's own refresh. Accepting it painted Running over Succeeded, re-enabled Hủy,
  // and restarted polling.
  it("ignores a stale lower-revision prop instead of repainting Running over a terminal state", async () => {
    vi.useFakeTimers();
    const terminal = detail({
      revision: 4,
      state: "succeeded",
      attempts: [attempt("attempt-a", "succeeded", false)],
    });
    const view = render(<FlowRunMonitor run={terminal} onCancel={vi.fn()} onRetry={vi.fn()} />);
    expect(screen.getAllByText("Thành công")).toHaveLength(2);

    view.rerender(
      <FlowRunMonitor run={detail({ revision: 2, state: "running" })} onCancel={vi.fn()} onRetry={vi.fn()} />,
    );

    expect(screen.getAllByText("Thành công")).toHaveLength(2);
    expect(screen.queryByText("Đang chạy")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Hủy/ })).toBeDisabled();
    // And polling stays stopped: a terminal state must not start the 750 ms interval.
    await act(async () => {
      vi.advanceTimersByTime(3_000);
      await Promise.resolve();
    });
    expect(api.flowGetRun).not.toHaveBeenCalled();
  });

  it("drops a stale refresh result whose revision is behind the screen", async () => {
    let handler: EventHandler | null = null;
    api.listenRiviuEvents.mockImplementation(async (next: EventHandler) => {
      handler = next;
      return vi.fn();
    });
    // A late, duplicated runtime event points the refresh at a projection the database has
    // since moved past; the fetch legitimately returns revision 2, running.
    api.flowGetRun.mockResolvedValue(detail({ revision: 2, state: "running" }));
    render(
      <FlowRunMonitor
        run={detail({
          revision: 4,
          state: "succeeded",
          attempts: [attempt("attempt-a", "succeeded", false)],
        })}
        onCancel={vi.fn()}
        onRetry={vi.fn()}
      />,
    );
    await waitFor(() => expect(api.listenRiviuEvents).toHaveBeenCalledTimes(1));

    await act(async () => {
      handler?.({ type: "flowRunUpdated", runId: "run-a", revision: 2 });
      await Promise.resolve();
    });

    expect(api.flowGetRun).toHaveBeenCalledTimes(1);
    expect(screen.getAllByText("Thành công")).toHaveLength(2);
    expect(screen.queryByText("Đang chạy")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Hủy/ })).toBeDisabled();
  });
});

describe("FlowRunMonitor says when it stopped being able to read the run", () => {
  it("reports a refresh failure instead of leaving a stale Running table", async () => {
    // `refresh` had a `finally` and no `catch`, so every rejection went to the global
    // unhandled-rejection handler while the table sat on the last projection it could read --
    // still saying Running, with nothing to say the numbers had stopped moving.
    vi.useFakeTimers();
    api.flowGetRun.mockRejectedValue(new Error("DeviceControl: bridge gone"));
    render(
      <FlowRunMonitor
        run={detail()}
        onCancel={vi.fn()}
        onRetry={vi.fn()}
      />,
    );
    await act(async () => {
      vi.advanceTimersByTime(800);
    });
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.getByRole("alert")).toHaveTextContent("Không đọc được tiến trình mới.");
    expect(screen.getByText("Không đọc được tiến trình mới.").closest("details")).toHaveTextContent(
      "DeviceControl: bridge gone",
    );
  });

  it("shows the message behind a run error, not only its code", () => {
    // The backend maps several distinct WDA and device failures onto one code and keeps what
    // separates them in the message, so a code-only cell hid the difference between a timeout, a
    // dead session, and the wrong app in the foreground.
    const stalled = detail();
    stalled.run = {
      ...stalled.run,
      state: "failed",
      error: {
        code: "DeviceControl",
        message: "phiên WDA đã chết",
        nodeId: null,
        field: null,
        udid: "device-a",
        attemptId: null,
      },
    };
    render(<FlowRunMonitor run={stalled} onCancel={vi.fn()} onRetry={vi.fn()} />);
    expect(screen.getByText("Lượt chạy gặp lỗi.").closest("details")).toHaveTextContent(
      "DeviceControl: phiên WDA đã chết",
    );
  });
});
