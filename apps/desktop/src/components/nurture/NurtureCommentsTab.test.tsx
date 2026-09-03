import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { NurtureCommentsTab } from "./NurtureCommentsTab";
import type { NurtureCommentAttempt } from "../../types";

const listCommentAttempts = vi.hoisted(() => vi.fn());
const costSummary = vi.hoisted(() => vi.fn());

// **Every export this component reaches for, named here.** An object-literal `vi.mock` returns
// `undefined` for anything it omits, and calling `undefined()` throws synchronously during
// render -- so one missing entry fails every test in the file at once, with a React stack
// instead of a message about the mock. That has now happened four times in this repo; adding
// `nurtureCostSummary` to the component without adding it here reddened all nine of these.
vi.mock("../../api", () => ({
  nurtureListCommentAttempts: listCommentAttempts,
  nurtureCostSummary: costSummary,
}));

function row(over: Partial<NurtureCommentAttempt>): NurtureCommentAttempt {
  return {
    id: "a1",
    udid: "ce0717171c2a64d50d",
    outcome: "sent",
    source: "grounded-vision",
    model: "deepseek-v4-flash-vision-exp",
    baseUrlHost: "api.deepseek.com",
    promptTokens: 475,
    completionTokens: 135,
    preview: "trà nguội thơm thật",
    captionPreview: "",
    frameSha256: "abc",
    contextConfidence: 80,
    relevance: 82,
    evidenceSupport: 40,
    distinctFrames: 1,
    carouselSlides: 0,
    createdAt: "2026-08-23T14:20:00Z",
    ...over,
  };
}

beforeEach(() => {
  // Default for the tests that are not about the totals: resolve with zeroes so the strip
  // renders and nothing throws.
  costSummary.mockReset();
  costSummary.mockResolvedValue({
    todayPromptTokens: 0,
    todayCompletionTokens: 0,
    totalPromptTokens: 0,
    totalCompletionTokens: 0,
    todayComments: 0,
    totalComments: 0,
  });
  // This project does not enable Testing Library's globals, so nothing unmounts between
  // cases on its own — and a leftover row from the previous case is exactly the kind of
  // pollution that makes a `queryByText(...).not` assertion here fail for the wrong reason.
  cleanup();
  listCommentAttempts.mockReset();
});

describe("the comment audit", () => {
  it("puts the frame count next to the evidence score, so a low score can be read", async () => {
    // The whole reason this panel exists: `bằng chứng 40` on a still photo post and on a
    // three-frame video are different findings, and until the count was rendered they read
    // identically.
    listCommentAttempts.mockResolvedValue([row({ distinctFrames: 1, evidenceSupport: 40 })]);
    render(<NurtureCommentsTab live={false} />);
    await waitFor(() =>
      expect(
        screen.getByText(/1 khung — bài tĩnh, không có chuyển động · bằng chứng 40\/100/),
      ).toBeInTheDocument(),
    );
  });

  it("uses the fleet machine label and keeps the raw UDID out of primary copy", async () => {
    listCommentAttempts.mockResolvedValue([row({})]);
    render(
      <NurtureCommentsTab
        live={false}
        deviceLabel={() => "Máy 7 · ONE-01"}
      />,
    );

    const label = await screen.findByText("Máy 7 · ONE-01");
    expect(label).toHaveAttribute("title", "ce0717171c2a64d50d");
    expect(screen.queryByText("a64d50d")).toBeNull();
  });

  it("shows a skip as a skip, with the caption it did read", async () => {
    // A skip is the more interesting row — it says the evidence was unusable or the verifier
    // refused the draft — and it carries no comment text at all.
    listCommentAttempts.mockResolvedValue([
      row({
        id: "a2",
        outcome: "context_skipped: comment_context_rejected: context=41",
        preview: "",
        captionPreview: "chào buổi sáng",
        distinctFrames: 3,
      }),
    ]);
    render(<NurtureCommentsTab live={false} />);
    await waitFor(() =>
      expect(screen.getByText("bỏ — bằng chứng không dùng được")).toBeInTheDocument(),
    );
    expect(screen.getByText("chú thích: chào buổi sáng")).toBeInTheDocument();
    expect(screen.getByText(/3 khung khác nhau/)).toBeInTheDocument();
  });

  it("reads the parked-stream case as the pair of numbers it is", async () => {
    // Seven slides paged, one picture handed back: the comment is grounded on a seventh of the
    // post, and neither number says that on its own.
    listCommentAttempts.mockResolvedValue([row({ carouselSlides: 7, distinctFrames: 1 })]);
    render(<NurtureCommentsTab live={false} />);
    await waitFor(() =>
      expect(
        screen.getByText(/1 khung — bài tĩnh, không có chuyển động · lướt 7 ảnh/),
      ).toBeInTheDocument(),
    );
  });

  it("says nothing about slides on a post that was never paged", async () => {
    // A video, or a build with no measured photo badge. `lướt 0 ảnh` there is noise.
    listCommentAttempts.mockResolvedValue([row({ carouselSlides: 0, distinctFrames: 3 })]);
    render(<NurtureCommentsTab live={false} />);
    await waitFor(() => expect(screen.getByText(/3 khung khác nhau/)).toBeInTheDocument());
    expect(screen.queryByText(/lướt/)).not.toBeInTheDocument();
  });

  it("names a comment that was charged for and never posted", async () => {
    listCommentAttempts.mockResolvedValue([
      row({ outcome: "deferred_card_changed", preview: "", carouselSlides: 4 }),
    ]);
    render(<NurtureCommentsTab live={false} />);
    await waitFor(() =>
      expect(screen.getByText("bỏ — thẻ đổi khi đang xem ảnh")).toBeInTheDocument(),
    );
  });

  it("maps card-changed audit outcomes without exposing the raw code as primary copy", async () => {
    listCommentAttempts.mockResolvedValue([
      row({ outcome: "skipped: card_changed", preview: "" }),
      row({ id: "a3", outcome: "engine_future_state", preview: "" }),
    ]);

    render(<NurtureCommentsTab live={false} />);

    await waitFor(() =>
      expect(screen.getByText("bỏ — thẻ đã đổi trước thao tác")).toBeInTheDocument(),
    );
    expect(screen.getByText("trạng thái chưa nhận diện")).toHaveAttribute(
      "title",
      "engine_future_state",
    );
    expect(screen.queryByText("engine_future_state")).not.toBeInTheDocument();
  });

  it("says nothing was recorded rather than showing an empty list", async () => {
    listCommentAttempts.mockResolvedValue([]);
    render(<NurtureCommentsTab live={false} />);
    await waitFor(() =>
      expect(screen.getByText(/Chưa có lượt bình luận nào được ghi/)).toBeInTheDocument(),
    );
  });

  it("does not claim a frame count for a row written before it was recorded", async () => {
    listCommentAttempts.mockResolvedValue([row({ distinctFrames: undefined })]);
    render(<NurtureCommentsTab live={false} />);
    await waitFor(() => expect(screen.getByText(/chưa ghi số khung/)).toBeInTheDocument());
  });

  it("polls only while a session is running", async () => {
    vi.useFakeTimers();
    try {
      listCommentAttempts.mockResolvedValue([row({})]);
      const { unmount } = render(<NurtureCommentsTab live={false} />);
      expect(listCommentAttempts).toHaveBeenCalledTimes(1);
      await vi.advanceTimersByTimeAsync(20_000);
      expect(listCommentAttempts).toHaveBeenCalledTimes(1);
      unmount();

      render(<NurtureCommentsTab live />);
      const before = listCommentAttempts.mock.calls.length;
      await vi.advanceTimersByTimeAsync(12_000);
      expect(listCommentAttempts.mock.calls.length).toBeGreaterThan(before + 1);
    } finally {
      vi.useRealTimers();
    }
  });

  it("shows the failure instead of an empty panel when the command fails", async () => {
    listCommentAttempts.mockRejectedValue(new Error("database is locked"));
    render(<NurtureCommentsTab live={false} />);
    await waitFor(() => expect(screen.getByText(/database is locked/)).toBeInTheDocument());
  });

  it("offers a retry after a list failure", async () => {
    listCommentAttempts
      .mockRejectedValueOnce(new Error("database is locked"))
      .mockResolvedValueOnce([row({ preview: "đã đọc lại" })]);
    render(<NurtureCommentsTab live={false} />);

    await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("database is locked"));
    fireEvent.click(screen.getByRole("button", { name: "Thử lại" }));
    await waitFor(() => expect(screen.getByText(/đã đọc lại/)).toBeInTheDocument());
  });

  /**
   * **The totals were registered, typed, allowlisted and rendered by nobody for months.**
   *
   * `nurture_cost_summary` reads the same table these rows come from and answers the one
   * question the rows cannot: how much has this cost. The repo had already written the lesson
   * down for its sibling command -- *"a number nobody reads cannot be checked"* -- and then left
   * this one in the same state.
   */
  it("shows tokens and comment counts for today and in total", async () => {
    listCommentAttempts.mockResolvedValue([row({})]);
    costSummary.mockResolvedValue({
      todayPromptTokens: 475,
      todayCompletionTokens: 135,
      totalPromptTokens: 12_000,
      totalCompletionTokens: 3_400,
      todayComments: 1,
      totalComments: 42,
    });

    render(<NurtureCommentsTab live={false} />);

    await waitFor(() => expect(screen.getByText("Hôm nay")).toBeInTheDocument());
    // 475 + 135, grouped the way vi-VN groups it.
    expect(screen.getByText(/610 token/)).toBeInTheDocument();
    expect(screen.getByText(/42 bình luận/)).toBeInTheDocument();
  });

  /**
   * **A broken total must not take the table down with it.**
   *
   * The rows are what an operator is reading. Failing the whole panel because an aggregate
   * query errored would trade the useful half for the decorative one.
   */
  it("still lists the rows when the totals fail", async () => {
    listCommentAttempts.mockResolvedValue([row({ preview: "quán này ngon" })]);
    costSummary.mockRejectedValue(new Error("aggregate failed"));

    render(<NurtureCommentsTab live={false} />);

    await waitFor(() => expect(screen.getByText(/quán này ngon/)).toBeInTheDocument());
    expect(screen.queryByText("Hôm nay")).toBeNull();
    expect(screen.getByRole("alert")).toHaveTextContent(/chưa đọc được tổng chi phí/i);
  });

  /**
   * An empty recent list still shows the totals, because "nothing lately" and "nothing ever"
   * are different facts and only the second one means the feature has never worked.
   */
  it("shows the totals even when no recent attempt was recorded", async () => {
    listCommentAttempts.mockResolvedValue([]);
    costSummary.mockResolvedValue({
      todayPromptTokens: 0,
      todayCompletionTokens: 0,
      totalPromptTokens: 12_000,
      totalCompletionTokens: 3_400,
      todayComments: 0,
      totalComments: 42,
    });

    render(<NurtureCommentsTab live={false} />);

    await waitFor(() => expect(screen.getByText(/42 bình luận/)).toBeInTheDocument());
    expect(screen.getByText(/Chưa có lượt bình luận nào/)).toBeInTheDocument();
  });
});
