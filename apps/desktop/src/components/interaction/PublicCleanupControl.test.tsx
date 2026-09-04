import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type {
  PublicCleanupCapability,
  PublicCleanupExecutionReport,
} from "../../types";
import { PublicCleanupControl } from "./PublicCleanupControl";

const { execute, preflight, confirm } = vi.hoisted(() => ({
  execute: vi.fn(),
  preflight: vi.fn(),
  confirm: vi.fn(),
}));

vi.mock("../../api", () => ({
  publicCleanupExecute: execute,
  publicCleanupPreflight: preflight,
}));

vi.mock("../../confirmStore", () => ({ requestConfirm: confirm }));

const ready: PublicCleanupCapability = {
  kind: "like",
  status: "readyForTargetProof",
  reason: "fresh canonical-card proof required",
  deviceUdid: "android-0",
  effectBoundaryCrossed: false,
};

const cleared: PublicCleanupExecutionReport = {
  capability: ready,
  run: {
    id: "cleanup-1",
    requestId: "request-cleanup-1",
    sourceActionRunId: "action-1",
    campaignId: "campaign-1",
    assignmentId: "assignment-1",
    deviceUdid: "android-0",
    kind: "like",
    targetJson: JSON.stringify({ contentId: "123", author: "creator" }),
    state: "cleared",
    revision: 3,
    effectIntent: "unlike_confirmed_source",
    evidence: "{}",
    error: null,
    updatedAt: "2026-09-05T01:00:00Z",
  },
  evidence: {
    verdict: "cleared",
    initial: {
      identity: {
        kind: "toggle",
        cardKey: "a".repeat(64),
        author: "creator",
        effect: "like",
      },
      sequence: 7,
      state: "present",
      tapPoint: { x: 900, y: 400 },
    },
    finalObservation: {
      identity: {
        kind: "toggle",
        cardKey: "a".repeat(64),
        author: "creator",
        effect: "like",
      },
      sequence: 8,
      state: "absent",
      tapPoint: { x: 900, y: 400 },
    },
    effectBoundaryCrossed: true,
    error: null,
  },
  sessionCleanupWarning: null,
};

function show(
  sourceState: "confirmed" | "uncertain" | "noOp" | "failedBeforeEffect" = "confirmed",
) {
  return render(
    <PublicCleanupControl
      campaignId="campaign-1"
      assignmentId="assignment-1"
      targetKey="content:123"
      actorLabel="2 · Máy canary · @creator"
      kind="like"
      sourceState={sourceState}
      sourceEvidence="source-card-digest"
    />,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  preflight.mockResolvedValue(ready);
  execute.mockResolvedValue(cleared);
  confirm.mockResolvedValue(true);
});

afterEach(() => vi.restoreAllMocks());

describe("PublicCleanupControl", () => {
  it("does no device work until the operator requests preflight", () => {
    show();
    expect(screen.getByRole("button", { name: "Kiểm tra bỏ tim" })).toBeEnabled();
    expect(preflight).not.toHaveBeenCalled();
    expect(execute).not.toHaveBeenCalled();
  });

  it("shows loading, then requires a separate confirmation before execute", async () => {
    let resolve!: (value: PublicCleanupCapability) => void;
    preflight.mockReturnValueOnce(new Promise((done) => { resolve = done; }));
    confirm.mockResolvedValueOnce(false);
    show();

    fireEvent.click(screen.getByRole("button", { name: "Kiểm tra bỏ tim" }));
    expect(screen.getByRole("status")).toHaveTextContent("Đang kiểm tra Tim");
    resolve(ready);

    const executeButton = await screen.findByRole("button", { name: "Xác nhận bỏ tim" });
    expect(execute).not.toHaveBeenCalled();
    fireEvent.click(executeButton);
    await waitFor(() => expect(confirm).toHaveBeenCalledWith(expect.objectContaining({
      title: "Bỏ Tim trên 2 · Máy canary · @creator?",
      danger: true,
    })));
    expect(execute).not.toHaveBeenCalled();
  });

  it("executes once after confirmation and keeps identity/readback inside details", async () => {
    show();
    fireEvent.click(screen.getByRole("button", { name: "Kiểm tra bỏ tim" }));
    fireEvent.click(await screen.findByRole("button", { name: "Xác nhận bỏ tim" }));

    await waitFor(() => expect(execute).toHaveBeenCalledTimes(1));
    expect(execute).toHaveBeenCalledWith(
      expect.any(String),
      "campaign-1",
      "assignment-1",
      "like",
    );
    expect(await screen.findByText("Đã bỏ Tim.")).toBeVisible();
    const details = screen.getByText("Danh tính đích và chứng cứ").closest("details");
    expect(details).not.toHaveAttribute("open");
    fireEvent.click(screen.getByText("Danh tính đích và chứng cứ"));
    expect(screen.getByText("content:123")).toBeVisible();
    expect(screen.getByText(/source-card-digest/)).toBeVisible();
    expect(screen.getByText(/"state": "absent"/)).toBeVisible();
    expect(screen.queryByRole("button", { name: /Kiểm tra lại/ })).toBeNull();
  });

  it("retries only a failed preflight", async () => {
    preflight
      .mockRejectedValueOnce(new Error("ADB đang bận"))
      .mockResolvedValueOnce(ready);
    show();

    fireEvent.click(screen.getByRole("button", { name: "Kiểm tra bỏ tim" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("ADB đang bận");
    fireEvent.click(screen.getByRole("button", { name: "Thử lại" }));
    expect(await screen.findByRole("button", { name: "Xác nhận bỏ tim" })).toBeEnabled();
    expect(preflight).toHaveBeenCalledTimes(2);
    expect(execute).not.toHaveBeenCalled();
  });

  it("blocks execute when preflight cannot prove the source", async () => {
    preflight.mockResolvedValueOnce({
      ...ready,
      status: "sourceNotConfirmed",
      reason: "source action was not confirmed",
    });
    show();

    fireEvent.click(screen.getByRole("button", { name: "Kiểm tra bỏ tim" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("đang bị khóa");
    expect(screen.queryByRole("button", { name: "Xác nhận bỏ tim" })).toBeNull();
    expect(screen.getByRole("button", { name: "Kiểm tra lại" })).toBeEnabled();
    expect(execute).not.toHaveBeenCalled();
  });

  it("disables cleanup when the source action is uncertain", () => {
    show("uncertain");
    expect(screen.getByRole("button", { name: "Bỏ Tim" })).toBeDisabled();
    expect(screen.getByText(/Kết quả nguồn chưa chắc chắn/)).toBeVisible();
    expect(preflight).not.toHaveBeenCalled();
  });

  it("treats a lost execute response as uncertain and exposes no retry", async () => {
    execute.mockRejectedValueOnce(new Error("response channel closed"));
    show();
    fireEvent.click(screen.getByRole("button", { name: "Kiểm tra bỏ tim" }));
    fireEvent.click(await screen.findByRole("button", { name: "Xác nhận bỏ tim" }));

    await waitFor(() =>
      expect(screen.getByRole("alert")).toHaveTextContent("Không xác định được kết quả"),
    );
    expect(screen.getByRole("alert")).toHaveTextContent("Không tự chạy lại");
    expect(screen.queryByRole("button", { name: /Kiểm tra lại|Xác nhận bỏ tim/ })).toBeNull();
  });

  it("locks an after-effect uncertain report instead of offering another tap", async () => {
    execute.mockResolvedValueOnce({
      ...cleared,
      run: { ...cleared.run!, state: "uncertain" },
      evidence: {
        ...cleared.evidence!,
        verdict: "notConfirmed",
        effectBoundaryCrossed: true,
      },
    });
    show();
    fireEvent.click(screen.getByRole("button", { name: "Kiểm tra bỏ tim" }));
    fireEvent.click(await screen.findByRole("button", { name: "Xác nhận bỏ tim" }));

    await waitFor(() =>
      expect(screen.getByRole("alert")).toHaveTextContent("Hoàn tác chưa hoàn tất"),
    );
    expect(screen.getByRole("alert")).toHaveTextContent("Đã thao tác nhưng chưa xác nhận");
    expect(screen.queryByRole("button", { name: /Kiểm tra lại|Xác nhận bỏ tim/ })).toBeNull();
  });
});
