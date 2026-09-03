import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { InteractionCampaignDetailView } from "./InteractionCampaignDetail";
import type {
  InteractionCampaignDetail,
  InteractionTargetNote,
} from "../../types";

/**
 * The web-lookup panel has exactly one job: keep three states apart that all look like an empty
 * row. AGENTS.md 9.103 §4 is about the version of this that had no screen at all; these are
 * about the version that has one and still says nothing useful.
 *
 * - **enriched** — a caption length, a slide count, a transcript track;
 * - **refused** — `errorCode`, which on this farm is two targets in seven (`ip_blocked`), and
 *   the phones can open those posts fine;
 * - **not looked up** — a campaign that predates the lookup, or one that is all manual.
 *
 * Rendered through the real detail view rather than a extracted panel, because the panel is
 * only reachable that way and a test that reaches it another way proves a different component.
 */
const detail: InteractionCampaignDetail = {
  summary: {
    id: "campaign-1",
    requestId: "req-1",
    state: "succeeded",
    messageCount: 1,
    targetCount: 3,
    succeededMessages: 3,
    failedMessages: 0,
    errorCode: null,
    updatedAt: "2026-08-26T00:00:00Z",
    brief: null,
  },
  assignments: [],
};

function note(over: Partial<InteractionTargetNote>): InteractionTargetNote {
  return {
    targetKey: "content:1",
    lineNo: 1,
    normalizedUrl: "https://www.tiktok.com/@a/video/1",
    kind: "video",
    captionChars: null,
    captionPreview: null,
    durationSecs: null,
    slideCount: null,
    hasOriginalAudio: null,
    subtitleLangs: [],
    transcriptTrack: null,
    errorCode: null,
    errorDetail: null,
    ...over,
  };
}

function show(notes: InteractionTargetNote[], shownDetail = detail) {
  render(
    <InteractionCampaignDetailView
      detail={shownDetail}
      artifacts={[]}
      notes={notes}
      devices={[]}
      deviceNumber={new Map()}
      handles={{}}
      busy={false}
      error={null}
      onBack={vi.fn()}
      onCancel={vi.fn()}
      onRetry={vi.fn()}
      onShowShot={vi.fn()}
      shot={null}
      onDismissShot={vi.fn()}
    />,
  );
}

// This project does not auto-clean between tests, so a leaked render turns the next
// `queryByText` into "multiple elements found" — which reads as the opposite of the absence it
// was asserting.
afterEach(() => {
  cleanup();
});

describe("web lookup panel", () => {
  it("shows the caption length, which is the measurement the phone cannot match", () => {
    show([
      note({
        captionChars: 105,
        captionPreview: "Cùng tớ khám phá lịch trình 1 ngày",
        durationSecs: 52,
      }),
    ]);
    // 105 fetched against the 76 the accessibility tree gave for the same post: the number is
    // the whole argument for the lookup, so it has to be on screen as a number.
    expect(screen.getByText("105 ký tự")).toBeInTheDocument();
    expect(screen.getByText("52s")).toBeInTheDocument();
  });

  it("names why a lookup was refused instead of leaving the row blank", () => {
    show([note({ errorCode: "ip_blocked", errorDetail: "chặn IP" })]);
    expect(screen.getByText(/TikTok chặn IP máy này/)).toBeInTheDocument();
    // And it is not mistaken for a target nobody looked at.
    expect(screen.queryByText("chưa tra")).not.toBeInTheDocument();
  });

  it("says a target was never looked up, rather than showing nothing", () => {
    show([note({})]);
    expect(screen.getByText("chưa tra")).toBeInTheDocument();
  });

  it("counts how many of the targets the lookup actually reached", () => {
    show([
      note({ targetKey: "content:1", lineNo: 1, captionChars: 105 }),
      note({ targetKey: "content:2", lineNo: 2, errorCode: "ip_blocked" }),
      note({ targetKey: "content:3", lineNo: 3 }),
    ]);
    // Two of three: the refused one *was* reached — it came back with a reason, which is a
    // finding. Only the blank row is a target nothing is known about.
    expect(screen.getByText("2/3 bài tra được")).toBeInTheDocument();
  });

  it("writes a dash for a video's slide count rather than zero", () => {
    show([note({ kind: "video", captionChars: 105 })]);
    // `slideCount` is null for every video; "0 ảnh" beside each of them is a number that means
    // nothing, and a table full of meaningless zeroes is how a panel stops being read.
    expect(screen.queryByText("0 ảnh")).not.toBeInTheDocument();
  });

  it("distinguishes a post with no speech from one nobody asked about", () => {
    show([
      note({ targetKey: "content:1", lineNo: 1, hasOriginalAudio: false, captionChars: 399 }),
      note({ targetKey: "content:2", lineNo: 2, transcriptTrack: "vie-VN/ASR", captionChars: 105 }),
    ]);
    // The measured reason a transcript was never fetched, not a shrug.
    expect(screen.getByText("nhạc nền")).toBeInTheDocument();
    expect(screen.getByText("vie-VN/ASR")).toBeInTheDocument();
  });

  it("renders nothing at all when there are no targets to describe", () => {
    show([]);
    expect(screen.queryByText(/bài tra được/)).not.toBeInTheDocument();
  });
});

describe("assignment evidence", () => {
  it("summarizes typed actions instead of calling a Like and Save run comments", () => {
    show([], {
      summary: {
        ...detail.summary,
        messageCount: 0,
        succeededMessages: 0,
        actionCounters: {
          planned: 2,
          attempted: 1,
          confirmed: 1,
          noOp: 1,
          uncertain: 0,
        },
      },
      assignments: [
        {
          id: "assignment-actions",
          targetKey: "content:1",
          ordinal: 0,
          actorUdid: "actor-a",
          parentAssignmentId: null,
          state: "succeeded",
          preparedText: null,
          errorCode: null,
          actions: [
            {
              kind: "like",
              state: "confirmed",
              revision: 2,
              effectIntent: "intent-like",
              evidence: "same-card-liked",
              error: null,
            },
            {
              kind: "save",
              state: "noOp",
              revision: 1,
              effectIntent: null,
              evidence: "already-saved",
              error: null,
            },
          ],
        },
      ],
      actionAggregate: "done",
    });

    expect(screen.getByLabelText("2 dự kiến")).toBeVisible();
    expect(screen.getByLabelText("1 đã thao tác")).toBeVisible();
    expect(screen.getByLabelText("1 xác nhận")).toBeVisible();
    expect(screen.getByLabelText("1 không cần làm")).toBeVisible();
    expect(screen.getByText("Tim · Đã xác nhận")).toBeVisible();
    expect(screen.getByText("Lưu · Không cần làm")).toBeVisible();
    expect(screen.queryByText(/0\/0 bình luận/)).not.toBeInTheDocument();
  });

  it("warns when a successful reply lives under a folded parent", () => {
    const foldedAssignment = {
      id: "assignment-folded",
      targetKey: "content:1",
      ordinal: 1,
      actorUdid: "actor-a",
      parentAssignmentId: "assignment-root",
      state: "succeeded" as const,
      preparedText: "Mình cũng thấy vậy",
      errorCode: null,
      parentWasFolded: true,
    };

    show([], { ...detail, assignments: [foldedAssignment] });

    expect(
      screen.getByText(
        "Bình luận cha bị TikTok gấp; phản hồi này đã gửi nhưng người khác không nhìn thấy.",
      ),
    ).toBeInTheDocument();
  });
});
