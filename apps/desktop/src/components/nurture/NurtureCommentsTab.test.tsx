import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { NurtureCommentsTab } from "./NurtureCommentsTab";
import type { NurtureCommentAttempt } from "../../types";

const listCommentAttempts = vi.hoisted(() => vi.fn());

vi.mock("../../api", () => ({
  nurtureListCommentAttempts: listCommentAttempts,
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
    expect(screen.getByText("caption: chào buổi sáng")).toBeInTheDocument();
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
});
