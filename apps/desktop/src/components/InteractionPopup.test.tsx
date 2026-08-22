import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { InteractionPopup } from "./InteractionPopup";
import type { ThreadCampaignRequest } from "../types";

const { parseLinks, startThread } = vi.hoisted(() => ({
  parseLinks: vi.fn(async () => [
    {
      lineNo: 1,
      original: "https://www.tiktok.com/@creator/video/123",
      target: {
        originalUrl: "https://www.tiktok.com/@creator/video/123",
        normalizedUrl: "https://www.tiktok.com/@creator/video/123",
        targetKey: "content:123",
        contentId: "123",
        author: "creator",
        kind: "video",
      },
      error: null,
    },
  ]),
  startThread: vi.fn(async () => ({
    queued: true,
    campaign: {
      id: "campaign-1",
      requestId: "request-1",
      state: "running",
      messageCount: 2,
      targetCount: 1,
      succeededMessages: 0,
      failedMessages: 0,
      updatedAt: "2026-08-04T00:00:00Z",
    },
  })),
}));

vi.mock("../api", () => ({
  interactionCancel: vi.fn(async () => undefined),
  interactionGet: vi.fn(async () => null),
  interactionList: vi.fn(async () => []),
  interactionParseLinks: parseLinks,
  interactionResolveLinks: vi.fn(async () => []),
  interactionStartThread: startThread,
  listenRiviuEvents: vi.fn(async () => () => undefined),
  listGroups: vi.fn(async () => []),
  // Reached on mount, once per in-scope device, to load each phone's @handle.
  getDeviceMeta: vi.fn(async (udid: string) => ({
    udid,
    notes: "",
    tags: [],
    groupId: null,
    proxyId: null,
    handle: "",
  })),
  saveDeviceMeta: vi.fn(async () => undefined),
  interactionRetry: vi.fn(async () => undefined),
  interactionListArtifacts: vi.fn(async () => []),
  interactionReadArtifact: vi.fn(async () => ""),
}));

afterEach(() => {
  // Explicit, not relying on the auto-cleanup: several tests in this file render the
  // same popup, and a leaked render turns every `getBy*` into "found multiple elements"
  // in whichever test happens to run next — a failure that points at the wrong test.
  cleanup();
  vi.clearAllMocks();
});

const devices = [
  {
    udid: "actor-a",
    name: "Phone A",
    model: "iPhone 8",
    platform: "ios",
    osVersion: "16.7.16",
    connection: "usb",
    status: "ready",
    wdaReady: true,
  },
  {
    udid: "actor-b",
    name: "Phone B",
    model: "iPhone 8",
    platform: "ios",
    osVersion: "16.7.15",
    connection: "usb",
    status: "ready",
    wdaReady: true,
  },
  {
    udid: "actor-android",
    name: "Phone C",
    model: "Redmi Note 12",
    platform: "android",
    osVersion: "15",
    connection: "usb",
    status: "ready",
    wdaReady: true,
  },
] as never[];

describe("InteractionPopup", () => {
  it("parses multiline links and submits every selected actor", async () => {
    render(<InteractionPopup devices={devices} selected={[]} onClose={() => undefined} />);
    const input = screen.getByPlaceholderText("https://www.tiktok.com/@creator/video/123");
    fireEvent.change(input, { target: { value: "https://www.tiktok.com/@creator/video/123" } });
    await waitFor(() => expect(screen.getByText(/creator\/video\/123/)).toBeVisible());

    fireEvent.click(screen.getByRole("button", { name: "Chạy ngay" }));
    await waitFor(() => expect(startThread).toHaveBeenCalledTimes(1));
    const request = (startThread.mock.calls as unknown as Array<[ThreadCampaignRequest]>)[0][0];
    expect(request.actorUdids).toEqual(["actor-a", "actor-b"]);
    expect(request.messageCount).toBe(2);
    expect(request.targets[0].targetKey).toBe("content:123");
    expect(screen.getByRole("tab", { name: "Monitor" })).toHaveAttribute("aria-selected", "true");
  });

  it("offers Android as an actor, grouped by how it reads the screen", async () => {
    // Android used to be filtered out here because the Interaction send path was
    // pixel-only. It now drives the drawer through the accessibility hierarchy, so
    // excluding it would hide working hardware. What replaces the filter is the grouping
    // plus the mixed-thread refusal below.
    render(<InteractionPopup devices={devices} selected={[]} onClose={() => undefined} />);
    expect(screen.getByLabelText("Phone C")).toBeVisible();
    expect(screen.getByText("iPhone (nhận dạng ảnh)")).toBeVisible();
    expect(screen.getByText("Android (hierarchy)")).toBeVisible();
    // Pre-selection comes from one group only, so the default is a runnable thread. Two
    // iPhones outnumber one Android here, so the iPhones win.
    expect(screen.getByRole("button", { name: "Chạy ngay" })).toBeEnabled();

    fireEvent.change(screen.getByPlaceholderText("https://www.tiktok.com/@creator/video/123"), {
      target: { value: "https://www.tiktok.com/@creator/video/123" },
    });
    await waitFor(() => expect(screen.getByText(/creator\/video\/123/)).toBeVisible());
    fireEvent.click(screen.getByRole("button", { name: "Chạy ngay" }));
    await waitFor(() => expect(startThread).toHaveBeenCalledTimes(1));
    const request = (startThread.mock.calls as unknown as Array<[ThreadCampaignRequest]>)[0][0];
    expect(request.actorUdids).toEqual(["actor-a", "actor-b"]);
  });

  it("keeps the operator's actor selection across a fleet poll", async () => {
    // The seeding effect depended on the actor lists, which are memos over `devices` -- a
    // fresh array every three seconds from the fleet poll. So `setActors` re-ran on every
    // tick and threw away whatever had just been chosen: selecting actors for a threaded
    // campaign was a race against the next poll, and nobody wins that. A re-render with an
    // equal-but-new `devices` array is exactly what the poll does.
    const { rerender } = render(
      <InteractionPopup devices={devices} selected={[]} onClose={() => undefined} />,
    );
    // Drop one of the two defaults; the operator's choice is now one iPhone.
    fireEvent.click(screen.getByLabelText("Phone B"));
    expect(screen.getByLabelText("Phone B")).not.toBeChecked();

    rerender(
      <InteractionPopup
        devices={devices.map((device) => ({ ...(device as object) })) as never[]}
        selected={[]}
        onClose={() => undefined}
      />,
    );

    expect(screen.getByLabelText("Phone B")).not.toBeChecked();
    expect(screen.getByLabelText("Phone A")).toBeChecked();
  });

  it("drops an actor that left the fleet without touching the others", async () => {
    const { rerender } = render(
      <InteractionPopup devices={devices} selected={[]} onClose={() => undefined} />,
    );
    expect(screen.getByLabelText("Phone A")).toBeChecked();
    expect(screen.getByLabelText("Phone B")).toBeChecked();

    rerender(
      <InteractionPopup
        devices={devices.filter((device) => (device as { udid: string }).udid !== "actor-b")}
        selected={[]}
        onClose={() => undefined}
      />,
    );

    expect(screen.queryByLabelText("Phone B")).toBeNull();
    expect(screen.getByLabelText("Phone A")).toBeChecked();
  });

  it("refuses a nested thread that mixes a pixel actor with a hierarchy actor", async () => {
    // The chain is linear and each message is sent from a different actor, so message N
    // must find message N-1's comment. The two readers get the author label from
    // different places and need not agree, which breaks the chain halfway with no
    // explanation. The server refuses this as well; this is the round trip saved and the
    // reason stated where the operator is looking.
    render(<InteractionPopup devices={devices} selected={[]} onClose={() => undefined} />);
    fireEvent.change(screen.getByPlaceholderText("https://www.tiktok.com/@creator/video/123"), {
      target: { value: "https://www.tiktok.com/@creator/video/123" },
    });
    await waitFor(() => expect(screen.getByText(/creator\/video\/123/)).toBeVisible());

    // Add the Android device to the two already-selected iPhones.
    fireEvent.click(screen.getByLabelText("Phone C"));
    await waitFor(() =>
      expect(screen.getByText(/không chạy trộn iPhone với Android/)).toBeVisible(),
    );
    expect(screen.getByRole("button", { name: "Chạy ngay" })).toBeDisabled();
    expect(startThread).not.toHaveBeenCalled();
  });

  it("allows the same mixed selection in Standalone, which has no parent to find", async () => {
    render(<InteractionPopup devices={devices} selected={[]} onClose={() => undefined} />);
    fireEvent.change(screen.getByPlaceholderText("https://www.tiktok.com/@creator/video/123"), {
      target: { value: "https://www.tiktok.com/@creator/video/123" },
    });
    await waitFor(() => expect(screen.getByText(/creator\/video\/123/)).toBeVisible());
    // One from each group, so the selection is genuinely mixed and still within the
    // default message budget (2 messages, 2 actors).
    fireEvent.click(screen.getByLabelText("Phone B"));
    fireEvent.click(screen.getByLabelText("Phone C"));
    // The mode is a `<select>`, not a radio — changing its value is what switches modes.
    fireEvent.change(screen.getByLabelText(/Kiểu tương tác/), {
      target: { value: "standalone" },
    });

    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Chạy ngay" })).toBeEnabled(),
    );
    fireEvent.click(screen.getByRole("button", { name: "Chạy ngay" }));
    await waitFor(() => expect(startThread).toHaveBeenCalledTimes(1));
    const request = (startThread.mock.calls as unknown as Array<[ThreadCampaignRequest]>)[0][0];
    expect(request.actorUdids).toEqual(["actor-a", "actor-android"]);
    expect(request.mode).toBe("standalone");
  });

  it("requires at least two actors before dispatch", async () => {
    render(<InteractionPopup devices={devices} selected={[]} onClose={() => undefined} />);
    fireEvent.change(screen.getByPlaceholderText("https://www.tiktok.com/@creator/video/123"), {
      target: { value: "https://www.tiktok.com/@creator/video/123" },
    });
    await waitFor(() => expect(screen.getByText(/creator\/video\/123/)).toBeVisible());
    fireEvent.click(screen.getByLabelText("Phone B"));
    fireEvent.click(screen.getByRole("button", { name: "Chạy ngay" }));
    await waitFor(() =>
      expect(screen.getByText(/Chọn từ 2 đến 64 thiết bị làm actor/)).toBeVisible(),
    );
    expect(startThread).not.toHaveBeenCalled();
  });

  it("sends the star shape, which is the one the operator asked for", async () => {
    // "Một máy bình luận gốc rồi các máy còn lại vào rep" — a star. The popup only ever
    // offered a chain, and a chain is a different thing: each account answers the one
    // before it, which is also why a chain cannot run in parallel.
    render(<InteractionPopup devices={devices} selected={[]} onClose={() => undefined} />);
    fireEvent.change(screen.getByPlaceholderText("https://www.tiktok.com/@creator/video/123"), {
      target: { value: "https://www.tiktok.com/@creator/video/123" },
    });
    await waitFor(() => expect(screen.getByText(/creator\/video\/123/)).toBeVisible());

    fireEvent.change(screen.getByLabelText(/^Hình chuỗi/), { target: { value: "star" } });
    fireEvent.click(screen.getByRole("button", { name: "Chạy ngay" }));

    await waitFor(() => expect(startThread).toHaveBeenCalledTimes(1));
    const request = (startThread.mock.calls as unknown as Array<[ThreadCampaignRequest]>)[0][0];
    expect(request.shape).toBe("star");
  });

  it("shows how the phones split into teams before anything runs", async () => {
    // The split is a decision the operator should see rather than discover from the
    // Monitor tab. Three phones in teams of two is one team of three, not a two and a
    // one: `partition_actors` spreads the remainder, and this line has to say the same
    // thing the backend will do.
    render(<InteractionPopup devices={devices} selected={[]} onClose={() => undefined} />);
    fireEvent.change(screen.getByLabelText("Cỡ cụm"), { target: { value: "2" } });

    const teams = await screen.findByTestId("cohort-preview");
    expect(teams.textContent).toContain("cụm 1");
    // Two iPhones are pre-selected, so teams of two is exactly one team.
    expect(teams.querySelectorAll("li")).toHaveLength(1);
  });

  it("no longer refuses a fleet larger than six", async () => {
    // The cap was 2..=6 on both sides, which is what made a twenty-phone run impossible
    // before anything else could go wrong.
    const fleet = Array.from({ length: 9 }, (_, index) => ({
      udid: `android-${index}`,
      name: `Android ${index}`,
      model: "SM-G955F",
      platform: "android",
      osVersion: "9",
      connection: "usb",
      status: "ready",
      wdaReady: true,
    })) as never[];
    render(<InteractionPopup devices={fleet} selected={[]} onClose={() => undefined} />);
    fireEvent.change(screen.getByPlaceholderText("https://www.tiktok.com/@creator/video/123"), {
      target: { value: "https://www.tiktok.com/@creator/video/123" },
    });
    await waitFor(() => expect(screen.getByText(/creator\/video\/123/)).toBeVisible());

    // Nine phones in teams of three: three messages a link covers the biggest team, which
    // is the rule that replaced "message count must cover the whole fleet".
    fireEvent.change(screen.getByLabelText("Cỡ cụm"), { target: { value: "3" } });
    fireEvent.change(screen.getByLabelText("Số message"), { target: { value: "3" } });
    fireEvent.click(screen.getByRole("button", { name: "Chạy ngay" }));

    await waitFor(() => expect(startThread).toHaveBeenCalledTimes(1));
    const request = (startThread.mock.calls as unknown as Array<[ThreadCampaignRequest]>)[0][0];
    expect(request.actorUdids).toHaveLength(9);
    expect(request.cohortSize).toBe(3);
    expect(request.messageCount).toBe(3);
  });

  it("groups the monitor by link, shows a refused like, and offers a retry", async () => {
    // Three things the Monitor could not do. Sixty rows from six teams running at once
    // interleave into an unreadable list; a like that was refused went only to the log; and
    // `interaction_retry` had existed since the feature shipped with nothing calling it.
    const api = await import("../api");
    vi.mocked(api.interactionList).mockResolvedValue([
      {
        id: "campaign-1",
        requestId: "request-1",
        state: "partial",
        messageCount: 2,
        targetCount: 2,
        succeededMessages: 3,
        failedMessages: 1,
        updatedAt: "2026-08-18T00:00:00Z",
      },
    ] as never);
    vi.mocked(api.interactionGet).mockResolvedValue({
      summary: {
        id: "campaign-1",
        requestId: "request-1",
        state: "partial",
        messageCount: 2,
        targetCount: 2,
        succeededMessages: 3,
        failedMessages: 1,
        updatedAt: "2026-08-18T00:00:00Z",
      },
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
          errorCode: "target_open_no_post_page",
        },
      ],
    } as never);

    render(<InteractionPopup devices={devices} selected={[]} onClose={() => undefined} />);
    fireEvent.click(screen.getByRole("tab", { name: "Monitor" }));
    fireEvent.click(await screen.findByText("request-1"));

    // One heading per link, because a link belongs to exactly one team.
    expect(await screen.findByText("link 111")).toBeVisible();
    expect(screen.getByText("link 222")).toBeVisible();
    expect(screen.getByText("2/2 message")).toBeVisible();
    expect(screen.getByText("0/1 message")).toBeVisible();

    // The like was refused while the comment posted: a note beside a succeeded row, not a
    // failure of it.
    expect(screen.getByText("không tim được: nhãn nút tim chưa đo")).toBeVisible();

    fireEvent.click(screen.getByRole("button", { name: "Thử lại phần hỏng" }));
    await waitFor(() => expect(api.interactionRetry).toHaveBeenCalledWith("campaign-1"));
  });
});
