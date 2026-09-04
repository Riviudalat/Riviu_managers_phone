import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { InteractionMonitorTab } from "./InteractionMonitorTab";
import type { AppEvent, InteractionCampaignSummary } from "../../types";

const { listeners } = vi.hoisted(() => ({ listeners: [] as ((event: AppEvent) => void)[] }));

vi.mock("../../api", () => ({
  interactionCancel: vi.fn(async () => undefined),
  interactionGet: vi.fn(async () => null),
  interactionList: vi.fn(async () => []),
  interactionListArtifacts: vi.fn(async () => []),
  // Added with the web-lookup panel. Missing it here does not fail loudly: an unmocked export
  // returns `undefined`, `.catch` on that throws synchronously inside `loadDetail`, and the
  // whole detail view silently stays on "Đang mở chiến dịch…" — which is how six tests in this
  // file went red at once.
  interactionListTargetNotes: vi.fn(async () => []),
  interactionReadArtifact: vi.fn(async () => ({
    id: "art-1",
    kind: "comment-root-evidence",
    mimeType: "image/jpeg",
    base64: "AAAA",
  })),
  publicCleanupExecute: vi.fn(),
  publicCleanupPreflight: vi.fn(),
  interactionRetry: vi.fn(async () => undefined),
  listenRiviuEvents: vi.fn(async (handler: (event: AppEvent) => void) => {
    listeners.push(handler);
    return () => {
      listeners.splice(listeners.indexOf(handler), 1);
    };
  }),
}));

afterEach(() => {
  cleanup();
  listeners.length = 0;
  vi.clearAllMocks();
});

const devices = [
  {
    udid: "android-0",
    name: "Máy Một",
    model: "SM-G955F",
    platform: "android",
    osVersion: "9",
    connection: "usb",
    status: "ready",
    wdaReady: true,
  },
] as never[];

const deviceNumber = new Map([["android-0", 7]]);

const summary = {
  id: "campaign-1",
  requestId: "request-1",
  state: "partial",
  messageCount: 2,
  targetCount: 2,
  succeededMessages: 3,
  failedMessages: 1,
  errorCode: null,
  updatedAt: "2026-08-18T00:00:00Z",
  brief: {
    firstAuthor: ".lt.gi.mang.v",
    firstContentId: "7668947001618320660",
    mode: "threaded",
    shape: "star",
    cohortSize: null,
    actorCount: 3,
    manual: false,
    likeTarget: true,
  },
};

const detail = {
  summary,
  assignments: [
    {
      id: "a1",
      targetKey: "content:111",
      ordinal: 0,
      actorUdid: "android-0",
      parentAssignmentId: null,
      state: "succeeded",
      preparedText: "gốc của cụm một",
      errorCode: null,
      like: "không tim được: nhãn nút tim chưa đo",
    },
    {
      id: "a2",
      targetKey: "content:111",
      ordinal: 1,
      actorUdid: "android-1",
      parentAssignmentId: "a1",
      state: "succeeded",
      preparedText: "rep của cụm một",
      errorCode: null,
    },
    {
      id: "a3",
      targetKey: "content:222",
      ordinal: 0,
      actorUdid: "android-3",
      parentAssignmentId: null,
      state: "failed",
      preparedText: "gốc của cụm hai",
      errorCode: "target_open_no_post_page: mở link xong không thấy trang bài",
    },
  ],
};

function renderTab(openCampaignId: string | null = null, onOpen = vi.fn()) {
  render(
    <InteractionMonitorTab
      devices={devices}
      deviceNumber={deviceNumber}
      handles={{ "android-0": "mangv" }}
      openCampaignId={openCampaignId}
      onOpenCampaign={onOpen}
    />,
  );
  return onOpen;
}

function renderMasterDetail(openCampaignId: string | null = null, onOpen = vi.fn()) {
  render(
    <InteractionMonitorTab
      devices={devices}
      deviceNumber={deviceNumber}
      handles={{ "android-0": "mangv" }}
      openCampaignId={openCampaignId}
      onOpenCampaign={onOpen}
      masterDetail
    />,
  );
  return onOpen;
}

describe("InteractionMonitorTab", () => {
  it.each([
    ["missing", null, "Chiến dịch không còn trong dữ liệu."],
    ["rejected", new Error("database read failed"), "database read failed"],
  ])("shows a retryable detail error when the campaign is %s", async (_, outcome, message) => {
    const api = await import("../../api");
    if (outcome instanceof Error) {
      vi.mocked(api.interactionGet).mockRejectedValueOnce(outcome);
    } else {
      vi.mocked(api.interactionGet).mockResolvedValueOnce(outcome);
    }
    vi.mocked(api.interactionGet).mockResolvedValueOnce(detail as never);
    renderMasterDetail("campaign-1");

    expect(await screen.findByRole("alert")).toHaveTextContent(message);
    expect(screen.queryByText("Đang mở chiến dịch…")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Thử lại" }));
    expect(await screen.findByText("link 111")).toBeVisible();
  });

  it("keeps the newest campaign-list response when an older request finishes later", async () => {
    const api = await import("../../api");
    let resolveOld!: (value: InteractionCampaignSummary[]) => void;
    vi.mocked(api.interactionList)
      .mockImplementationOnce(() => new Promise((resolve) => { resolveOld = resolve; }))
      .mockResolvedValueOnce([{ ...summary, id: "campaign-new", brief: { ...summary.brief, firstAuthor: "new.author" } }] as never);

    renderTab();
    await waitFor(() => expect(listeners).toHaveLength(1));
    listeners[0]({ type: "interactionUpdated", campaignId: "campaign-new", revision: 2 } as AppEvent);
    expect(await screen.findByText("@new.author +1 link")).toBeVisible();

    resolveOld([{ ...summary, id: "campaign-old", brief: { ...summary.brief, firstAuthor: "old.author" } }] as InteractionCampaignSummary[]);
    await Promise.resolve();
    expect(screen.getByText("@new.author +1 link")).toBeVisible();
    expect(screen.queryByText("@old.author +1 link")).toBeNull();
  });

  it("counts failed-before-effect actions as terminal and renders the typed aggregate", async () => {
    const api = await import("../../api");
    vi.mocked(api.interactionGet).mockResolvedValue({
      ...detail,
      summary: {
        ...summary,
        actionCounters: { planned: 3, attempted: 1, confirmed: 1, noOp: 0, uncertain: 0 },
      },
      actionAggregate: "partial",
      assignments: [{
        ...detail.assignments[0],
        actions: [
          { kind: "like", state: "confirmed", revision: 1, effectIntent: "tap", evidence: null, error: null },
          { kind: "save", state: "failedBeforeEffect", revision: 1, effectIntent: null, evidence: null, error: "card changed" },
          { kind: "comment", state: "failedBeforeEffect", revision: 1, effectIntent: null, evidence: null, error: "audit unavailable" },
        ],
      }],
    } as never);
    renderMasterDetail("campaign-1");

    expect(await screen.findByText("Hành động xong một phần")).toBeVisible();
    expect(screen.getByText(/3\/3 hành động đã có kết quả/)).toBeVisible();
    expect(screen.getByLabelText("2 chưa thực hiện")).toBeVisible();
    expect(screen.getByRole("progressbar", { name: "Tiến trình chiến dịch đang xem" }))
      .toHaveAttribute("aria-valuenow", "100");
    expect(screen.getByRole("button", { name: "Kiểm tra bỏ tim" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Bỏ Lưu" })).toBeDisabled();
    expect(screen.queryByRole("button", { name: /hoàn tác.*bình luận/i })).toBeNull();
  });

  it("closes list progress for terminal actions that failed before effect", async () => {
    const api = await import("../../api");
    vi.mocked(api.interactionList).mockResolvedValue([{
      ...summary,
      actionCounters: { planned: 4, attempted: 1, confirmed: 1, noOp: 1, uncertain: 0 },
    }] as never);
    renderTab();

    expect(await screen.findByText(/4\/4 hành động/)).toHaveTextContent("2 chưa thực hiện");
    expect(screen.getByRole("progressbar", { name: /Tiến trình/ }))
      .toHaveAttribute("aria-valuenow", "100");
  });

  it("keeps the campaign list beside the selected campaign on the page workspace", async () => {
    const api = await import("../../api");
    vi.mocked(api.interactionList).mockResolvedValue([summary] as never);
    vi.mocked(api.interactionGet).mockResolvedValue(detail as never);
    renderMasterDetail("campaign-1");

    expect(await screen.findByRole("region", { name: "Danh sách chiến dịch" })).toBeVisible();
    expect(screen.getByRole("complementary", { name: "Chi tiết chiến dịch" })).toBeVisible();
    expect(await screen.findByText("@.lt.gi.mang.v +1 link")).toBeVisible();
    expect(await screen.findByText("link 111")).toBeVisible();
  });

  it("distinguishes loading, load failure with retry, and a genuinely empty list", async () => {
    const api = await import("../../api");
    let rejectFirst!: (reason: Error) => void;
    vi.mocked(api.interactionList).mockImplementationOnce(
      () => new Promise((_, reject) => {
        rejectFirst = reject;
      }),
    );

    renderTab();
    expect(screen.getByText("Đang tải chiến dịch…")).toBeVisible();
    expect(screen.queryByText("Chưa có chiến dịch nào")).toBeNull();

    rejectFirst(new Error("không đọc được tương tác"));
    expect(await screen.findByRole("alert")).toHaveTextContent("không đọc được tương tác");
    expect(screen.queryByText("Chưa có chiến dịch nào")).toBeNull();

    vi.mocked(api.interactionList).mockResolvedValueOnce([]);
    fireEvent.click(screen.getByRole("button", { name: "Thử lại" }));
    expect(await screen.findByText("Chưa có chiến dịch nào")).toBeVisible();
  });

  it("names a campaign by its post instead of a slice of its UUID", async () => {
    const api = await import("../../api");
    vi.mocked(api.interactionList).mockResolvedValue([summary] as never);
    renderTab();
    expect(await screen.findByText("@.lt.gi.mang.v +1 link")).toBeVisible();
    // The counts and the state, in Vietnamese, plus the time that was on every summary and
    // rendered nowhere.
    expect(screen.getByText(/3\/4 bình luận · 1 lỗi/)).toBeVisible();
    expect(screen.getByText("Xong một phần")).toBeVisible();
    expect(screen.getByRole("progressbar", { name: /Tiến trình/ })).toBeVisible();
  });

  it("groups the detail by link, shows a refused like, and retries the broken part", async () => {
    // Sixty rows from six teams running at once interleave into an unreadable list; a like
    // that was refused went only to the log; and `interaction_retry` had existed since the
    // feature shipped with nothing calling it.
    const api = await import("../../api");
    vi.mocked(api.interactionGet).mockResolvedValue(detail as never);
    renderTab("campaign-1");

    expect(await screen.findByText("link 111")).toBeVisible();
    expect(screen.getByText("link 222")).toBeVisible();
    expect(screen.getByText("2/2 lượt")).toBeVisible();
    expect(screen.getByText("0/1 lượt")).toBeVisible();
    expect(screen.getByText("không tim được: nhãn nút tim chưa đo")).toBeVisible();

    fireEvent.click(screen.getByRole("button", { name: "Thử lại phần hỏng" }));
    await waitFor(() =>
      expect(api.interactionRetry).toHaveBeenCalledWith("campaign-1", undefined),
    );
  });

  it("names the actor by its tile number and handle, not by eight characters of a udid", async () => {
    const api = await import("../../api");
    vi.mocked(api.interactionGet).mockResolvedValue(detail as never);
    renderTab("campaign-1");
    expect(await screen.findByText("7 · Máy Một · @mangv")).toBeVisible();
    // Departed phones keep stable positions in this campaign without putting a raw serial on
    // the main surface. The serial remains available as technical hover/detail evidence.
    expect(screen.getByText("Máy đã rời fleet 1/2")).not.toHaveAttribute("title");
    expect(screen.getByText("Máy đã rời fleet 2/2")).not.toHaveAttribute("title");
    expect(screen.getByText("android-1").closest("details")).not.toHaveAttribute("open");
    expect(screen.getByText("android-3").closest("details")).not.toHaveAttribute("open");
  });

  it("translates a refusal and still keeps the raw code for a bug report", async () => {
    const api = await import("../../api");
    vi.mocked(api.interactionGet).mockResolvedValue(detail as never);
    renderTab("campaign-1");
    expect(await screen.findByText("Không thấy trang bài viết")).toBeVisible();
    const errorDetail = screen.getByText("mở link xong không thấy trang bài");
    expect(errorDetail).not.toBeVisible();
    expect(errorDetail.closest("details")).not.toHaveAttribute("open");
    // Behind a closed disclosure — present for whoever needs it, not competing with the
    // Vietnamese for the operator's attention.
    const rawCode = screen.getByText(/^target_open_no_post_page:/);
    expect(rawCode).toBeInTheDocument();
    expect(rawCode.closest("details")).not.toHaveAttribute("open");
  });

  it("retries one message on its own", async () => {
    // The backend has taken `assignmentIds` since the feature shipped and the UI only ever
    // asked for all of them, so repairing one dead phone re-ran every retryable message.
    const api = await import("../../api");
    vi.mocked(api.interactionGet).mockResolvedValue(detail as never);
    renderTab("campaign-1");
    const rowRetries = await screen.findAllByRole("button", { name: "Thử lại" });
    // Only the failed row offers one — the two succeeded rows must not.
    expect(rowRetries).toHaveLength(1);
    fireEvent.click(rowRetries[0]);
    await waitFor(() => expect(api.interactionRetry).toHaveBeenCalledWith("campaign-1", ["a3"]));
  });

  it("offers no per-row retry while the campaign is still working", async () => {
    // A queued message has not failed, and asking the engine to re-plan a campaign that is
    // still working through the first plan is not a repair.
    const api = await import("../../api");
    vi.mocked(api.interactionGet).mockResolvedValue({
      summary: { ...summary, state: "running" },
      assignments: [
        { ...detail.assignments[2], state: "failed" },
        { ...detail.assignments[1], id: "a9", state: "queued", preparedText: null },
      ],
    } as never);
    renderTab("campaign-1");
    await screen.findByText("link 222");
    expect(screen.queryByRole("button", { name: "Thử lại" })).toBeNull();
  });

  it("waits for Dừng and says so when it is refused", async () => {
    // It was fire-and-forget: no await, no busy state, no catch — so a cancel the backend
    // refused looked exactly like one it accepted.
    const api = await import("../../api");
    vi.mocked(api.interactionGet).mockResolvedValue({
      ...detail,
      summary: { ...summary, state: "running" },
    } as never);
    vi.mocked(api.interactionCancel).mockRejectedValue(new Error("máy đang bận"));
    renderTab("campaign-1");
    fireEvent.click(await screen.findByRole("button", { name: "Dừng" }));
    await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent(/máy đang bận/));
  });

  it("refreshes the open campaign and its artifacts when the backend says it changed", async () => {
    // Artifacts were fetched once when the campaign was opened and never again, so an
    // evidence frame saved mid-run only appeared after closing and reopening it.
    const api = await import("../../api");
    vi.mocked(api.interactionGet).mockResolvedValue(detail as never);
    renderTab("campaign-1");
    await waitFor(() => expect(api.interactionListArtifacts).toHaveBeenCalledTimes(1));

    listeners.forEach((handler) =>
      handler({ type: "interactionUpdated", campaignId: "campaign-1", revision: 2 } as AppEvent),
    );
    await waitFor(() => expect(api.interactionListArtifacts).toHaveBeenCalledTimes(2));
    expect(api.interactionList).toHaveBeenCalledTimes(2);
  });

  it("ignores an event for a campaign that is not open", async () => {
    const api = await import("../../api");
    vi.mocked(api.interactionGet).mockResolvedValue(detail as never);
    renderTab("campaign-1");
    await waitFor(() => expect(api.interactionGet).toHaveBeenCalledTimes(1));

    listeners.forEach((handler) =>
      handler({ type: "interactionUpdated", campaignId: "campaign-other", revision: 2 } as AppEvent),
    );
    await waitFor(() => expect(api.interactionList).toHaveBeenCalledTimes(2));
    expect(api.interactionGet).toHaveBeenCalledTimes(1);
  });

  it("subscribes once and unsubscribes on unmount", async () => {
    // The old effect depended on the open campaign id, so it tore down and re-subscribed on
    // every navigation — and `listen` is a promise, so an unmount before it resolved left the
    // listener attached with nothing to remove it.
    const { unmount } = render(
      <InteractionMonitorTab
        devices={devices}
        deviceNumber={deviceNumber}
        handles={{}}
        openCampaignId={null}
        onOpenCampaign={() => undefined}
      />,
    );
    await waitFor(() => expect(listeners).toHaveLength(1));
    unmount();
    await waitFor(() => expect(listeners).toHaveLength(0));
  });
});
