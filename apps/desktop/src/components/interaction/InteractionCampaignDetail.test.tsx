import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { InteractionCampaignDetailView } from "./InteractionCampaignDetail";
import type {
  InteractionActionState,
  InteractionAssignmentRecord,
  InteractionCampaignDetail,
  PublicActionResult,
} from "../../types";

vi.mock("./PublicCleanupControl", () => ({ PublicCleanupControl: () => null }));
vi.mock("./InteractionReadbackControl", () => ({ InteractionReadbackControl: () => null }));

afterEach(cleanup);

function action(kind: PublicActionResult["kind"], state: InteractionActionState, effectIntent: string | null = null): PublicActionResult {
  return { kind, state, effectIntent, revision: 1, evidence: null, error: null };
}

function assignment(index: number, state: InteractionAssignmentRecord["state"], actions: PublicActionResult[]): InteractionAssignmentRecord {
  return {
    id: `assignment-${index}`, targetKey: "content:111", ordinal: index, actorUdid: `device-${index}`,
    parentAssignmentId: null, state, preparedText: null, errorCode: null, actions,
  };
}

function campaign(assignments: InteractionAssignmentRecord[]): InteractionCampaignDetail {
  const actions = assignments.flatMap((record) => record.actions ?? []);
  return {
    summary: {
      id: "campaign-fixture", requestId: "request-fixture", state: "partial", messageCount: assignments.length,
      targetCount: 1, succeededMessages: assignments.filter((row) => row.state === "succeeded").length,
      failedMessages: assignments.filter((row) => row.state === "failed").length,
      errorCode: null, updatedAt: "2026-09-07T00:00:00Z", brief: null,
      actionCounters: {
        planned: actions.length,
        attempted: actions.filter((item) => item.effectIntent !== null).length,
        confirmed: actions.filter((item) => item.state === "confirmed").length,
        noOp: actions.filter((item) => item.state === "noOp").length,
        uncertain: actions.filter((item) => item.state === "uncertain").length,
      },
    },
    assignments,
    actionAggregate: "partial",
  };
}

function renderDetail(detail: InteractionCampaignDetail) {
  return render(<InteractionCampaignDetailView
    detail={detail} artifacts={[]} notes={[]} devices={[]} deviceNumber={new Map()} handles={{}}
    busy={false} error={null} onBack={() => {}} onCancel={() => {}} onRetry={() => {}}
    onShowShot={() => {}} shot={null} onDismissShot={() => {}}
  />);
}

describe("InteractionCampaignDetail terminal action projection", () => {
  it("shows compact machine outcomes and opens evidence only for the selected machine", () => {
    const detail = campaign([
      { ...assignment(0, "uncertain", [action("save", "uncertain", "tap")]), preparedText: "evidence for first machine" },
      { ...assignment(1, "succeeded", [action("comment", "confirmed", "send")]), preparedText: "evidence for second machine" },
    ]);
    const retry = vi.fn();
    render(<InteractionCampaignDetailView compact detail={detail} artifacts={[]} notes={[]} devices={[]} deviceNumber={new Map()} handles={{}}
      busy={false} error={null} onBack={() => {}} onCancel={() => {}} onRetry={retry}
      onShowShot={() => {}} shot={null} onDismissShot={() => {}} />);
    expect(screen.getByText("Lưu · Chưa chắc kết quả")).toBeVisible();
    expect(screen.queryByText("evidence for first machine")).toBeNull();
    fireEvent.click(screen.getAllByRole("button", { name: /^Xem log / })[0]);
    const drawer = screen.getByRole("dialog");
    expect(within(drawer).getByText("evidence for first machine")).toBeVisible();
    expect(within(drawer).queryByText("evidence for second machine")).toBeNull();
    expect(within(drawer).queryByRole("button", { name: "Thử lại" })).toBeNull();
    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(retry).not.toHaveBeenCalled();
  });
  it("settles all 40 actions while preserving 9 unclaimed historical comments and their raw records", () => {
    const rows = Array.from({ length: 20 }, (_, index) => {
      if (index < 11) {
        return assignment(index, "succeeded", [
          action("like", index === 10 ? "noOp" : "confirmed", index === 10 ? null : "tap"),
          action("comment", "confirmed", "send"),
        ]);
      }
      const error = index < 13 ? "target_open_no_baseline" : index < 19 ? "target_open_screen_unchanged" : "target_open_no_post_page";
      return {
        ...assignment(index, "failed", [
          { ...action("like", "failedBeforeEffect"), error },
          action("comment", "planned"),
        ]),
        errorCode: "Like không an toàn để tiếp tục assignment",
      };
    });
    const fixture = campaign(rows);
    const stored = structuredClone(fixture);
    renderDetail(fixture);

    expect(screen.getByText(/40\/40 hành động đã có kết quả/)).toBeVisible();
    expect(screen.getByLabelText("18 chưa thực hiện")).toBeVisible();
    expect(screen.getByLabelText("21 đã thao tác")).toBeVisible();
    expect(screen.getByLabelText("21 xác nhận")).toBeVisible();
    expect(screen.getByLabelText("1 không cần làm")).toBeVisible();
    expect(screen.getByRole("progressbar", { name: "Tiến trình chiến dịch đang xem" })).toHaveAttribute("aria-valuenow", "100");
    expect(screen.getAllByText("Bình luận · Chưa thực hiện: lượt đã dừng")).toHaveLength(9);
    expect(screen.queryByText("Bình luận · Đang chờ")).toBeNull();
    const rawAction = screen.getAllByText(JSON.stringify(action("comment", "planned")))[0];
    expect(rawAction.closest("details")).not.toHaveAttribute("open");
    expect(fixture).toEqual(stored);
  });

  it.each(["failed", "uncertain", "skippedParent"] as const)("projects only an unclaimed planned action on a %s parent", (parentState) => {
    renderDetail(campaign([assignment(0, parentState, [action("comment", "planned")])]));
    expect(screen.getByText("Bình luận · Chưa thực hiện: lượt đã dừng")).toBeVisible();
    expect(screen.getByLabelText("1 chưa thực hiện")).toBeVisible();
  });

  it.each([
    ["preparing", null, "Đang chuẩn bị"],
    ["preparing", "claim", "Đang chuẩn bị"],
    ["armed", "send", "Đã ghi ý định, chờ xác nhận"],
    ["planned", "send", "Đang chờ"],
    ["planned", undefined, "Đang chờ"],
  ] as const)("does not project state %s with intent %s as unperformed", (state, intent, label) => {
    const record = { ...action("comment", state), effectIntent: intent } as PublicActionResult;
    renderDetail(campaign([assignment(0, "failed", [record])]));
    expect(screen.getByText(`Bình luận · ${label}`)).toBeVisible();
    expect(screen.queryByLabelText("1 chưa thực hiện")).toBeNull();
    expect(screen.getByRole("progressbar", { name: "Tiến trình chiến dịch đang xem" })).toHaveAttribute("aria-valuenow", "0");
  });

  it.each(["queued", "preparing", "ready", "sending", "succeeded"] as const)("does not project planned actions on a %s parent", (parentState) => {
    renderDetail(campaign([assignment(0, parentState, [action("comment", "planned")])]));
    expect(screen.getByText("Bình luận · Đang chờ")).toBeVisible();
    expect(screen.queryByLabelText("1 chưa thực hiện")).toBeNull();
  });

  it("shows the precise failed action reason and keeps the generic parent wrapper only in details", () => {
    const row = assignment(0, "failed", [
      { ...action("like", "failedBeforeEffect"), error: "target_open_no_post_page: bài đích chưa mở" },
      action("comment", "planned"),
    ]);
    row.errorCode = "Like không an toàn để tiếp tục assignment";
    renderDetail(campaign([row]));
    expect(screen.getByText("Không thấy trang bài viết")).toBeVisible();
    expect(screen.queryByText("Lượt dừng ở bước Tim; xem nguyên nhân từng hành động")).toBeNull();
    const original = screen.getByLabelText("Mã lỗi lượt gốc");
    expect(original).not.toHaveAttribute("open");
    expect(within(original).getByText(row.errorCode)).not.toBeVisible();
    expect(screen.getAllByText("target_open_no_post_page: bài đích chưa mở")).toHaveLength(2);
  });

  it("keeps an independent assignment failure instead of replacing it with an action error", () => {
    const row = assignment(0, "failed", [{ ...action("like", "failedBeforeEffect"), error: "target_open_no_baseline" }]);
    row.errorCode = "target_open_no_post_page";
    renderDetail(campaign([row]));
    expect(screen.getByText("Không thấy trang bài viết")).toBeVisible();
    expect(screen.queryByLabelText("Mã lỗi lượt gốc")).toBeNull();
  });
});
