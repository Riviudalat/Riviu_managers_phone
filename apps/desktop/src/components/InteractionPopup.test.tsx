import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { InteractionPopup } from "./InteractionPopup";
import type { ThreadCampaignRequest, ThreadPlanAssignment } from "../types";

const { parseLinks, startThread, previewThread } = vi.hoisted(() => ({
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
      brief: null,
    },
  })),
  // The plan preview is the backend's own planner now, so every render with enough of a
  // draft reaches for it. The stand-in splits actors the way `partition_actors` does —
  // spreading the remainder — because the popup reads `largestCohort` off this answer and a
  // mock that always returns one team would validate against the wrong number.
  previewThread: vi.fn(async (request: ThreadCampaignRequest) => {
    // `plan_threads` runs the same `validate()` the dispatch will, so a preview carrying a
    // comment list shorter than its own message count is refused — `TooFewManualComments`.
    // Modelled here because leaving it out is what hid a deadlock: the panel guessed the fleet
    // count, the preview was refused against that guess, so the real cohort size never arrived
    // and the guess never improved.
    const manual = request.manualComments ?? [];
    if (manual.length > 0 && manual.length < request.messageCount) {
      throw new Error("InteractionFailed: manual mode needs as many comments as messages");
    }
    const size = request.cohortSize ?? 0;
    const teams = size >= 2 ? Math.max(1, Math.floor(request.actorUdids.length / size)) : 1;
    const base = Math.floor(request.actorUdids.length / teams);
    let remainder = request.actorUdids.length % teams;
    const assignments: ThreadPlanAssignment[] = [];
    let at = 0;
    for (let team = 0; team < teams; team += 1) {
      const take = base + (remainder > 0 ? 1 : 0);
      if (remainder > 0) remainder -= 1;
      request.actorUdids.slice(at, at + take).forEach((actorUdid, index) => {
        assignments.push({
          targetKey: request.targets[0]?.targetKey ?? "content:123",
          ordinal: index,
          actorUdid,
          parentOrdinal: index === 0 ? null : 0,
          cohort: team,
        });
      });
      at += take;
    }
    return {
      lines: [],
      validTargetCount: request.targets.length,
      cohortCount: teams,
      streamCapacity: 8,
      plan: { requestId: request.requestId, assignments },
    };
  }),
}));

vi.mock("../api", () => ({
  interactionCancel: vi.fn(async () => undefined),
  interactionGet: vi.fn(async () => null),
  interactionList: vi.fn(async () => []),
  interactionParseLinks: parseLinks,
  interactionPreviewThread: previewThread,
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

/**
 * Paste the one link the parse mock knows, and wait until it has actually been parsed.
 *
 * The wait is on the ✓ marker in the link list, not on the URL text: React mirrors a
 * textarea's value into its child text, so matching the URL matches what was just typed and
 * resolves before the (debounced) parse has returned anything.
 */
async function pasteLink() {
  fireEvent.change(screen.getByPlaceholderText("https://www.tiktok.com/@creator/video/123"), {
    target: { value: "https://www.tiktok.com/@creator/video/123" },
  });
  await screen.findByText("✓");
}

/**
 * Wait for the panel to be ready, then press Chạy ngay.
 *
 * The waiting is the point. `largestCohort` is read out of the last preview and the preview is
 * 350 ms behind the draft, so a click inside that gap dispatches a message count computed for a
 * different selection — thirteen messages for fourteen actors, which the planner refuses. The
 * panel now holds the button while the plan catches up, and a test that clicks without waiting
 * is testing a race the operator cannot win either.
 */
async function clickRun() {
  await waitFor(() => expect(screen.getByRole("button", { name: "Chạy ngay" })).toBeEnabled());
  fireEvent.click(screen.getByRole("button", { name: "Chạy ngay" }));
}

/** Open the collapsed advanced group, where the three numbers live. */
function openAdvanced() {
  fireEvent.click(screen.getByRole("button", { name: "Tuỳ chỉnh nâng cao" }));
}

describe("InteractionPopup", () => {
  it("parses multiline links and submits every selected actor", async () => {
    render(<InteractionPopup devices={devices} selected={[]} onClose={() => undefined} />);
    await pasteLink();

    await clickRun();
    await waitFor(() => expect(startThread).toHaveBeenCalledTimes(1));
    const request = (startThread.mock.calls as unknown as Array<[ThreadCampaignRequest]>)[0][0];
    expect(request.actorUdids).toEqual(["actor-a", "actor-b"]);
    expect(request.messageCount).toBe(2);
    expect(request.targets[0].targetKey).toBe("content:123");
    expect(screen.getByRole("tab", { name: "Theo dõi" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });

  it("opens the campaign it just started instead of dropping the operator in a list", async () => {
    // `interactionStartThread` returns the campaign and the popup used to throw it away, so
    // finding your own run meant recognising a slice of its UUID among the others.
    const api = await import("../api");
    render(<InteractionPopup devices={devices} selected={[]} onClose={() => undefined} />);
    await pasteLink();
    await clickRun();
    await waitFor(() => expect(api.interactionGet).toHaveBeenCalledWith("campaign-1"));
  });

  it("offers Android as an actor, grouped by how it reads the screen", async () => {
    // Android used to be filtered out here because the Interaction send path was
    // pixel-only. It now drives the drawer through the accessibility hierarchy, so
    // excluding it would hide working hardware. What replaces the filter is the grouping
    // plus the mixed-thread refusal below.
    render(<InteractionPopup devices={devices} selected={[]} onClose={() => undefined} />);
    expect(screen.getByLabelText("Phone C")).toBeVisible();
    expect(screen.getByText("iPhone (nhận dạng ảnh)")).toBeVisible();
    expect(screen.getByText("Android (đọc cây giao diện)")).toBeVisible();

    await pasteLink();
    await clickRun();
    await waitFor(() => expect(startThread).toHaveBeenCalledTimes(1));
    const request = (startThread.mock.calls as unknown as Array<[ThreadCampaignRequest]>)[0][0];
    // Pre-selection comes from one group only, so the default is a runnable thread. Two
    // iPhones outnumber one Android here, so the iPhones win.
    expect(request.actorUdids).toEqual(["actor-a", "actor-b"]);
  });

  it("keeps the operator's actor selection across a fleet poll", async () => {
    // The seeding effect depended on the actor lists, which are memos over `devices` -- a
    // fresh array every three seconds from the fleet poll. So the selection re-ran on every
    // tick and threw away whatever had just been chosen: selecting actors for a threaded
    // campaign was a race against the next poll, and nobody wins that.
    const { rerender } = render(
      <InteractionPopup devices={devices} selected={[]} onClose={() => undefined} />,
    );
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
    await pasteLink();

    // Add the Android device to the two already-selected iPhones.
    fireEvent.click(screen.getByLabelText("Phone C"));
    await waitFor(() =>
      expect(screen.getByText(/không chạy trộn iPhone với Android/)).toBeVisible(),
    );
    expect(screen.getByRole("button", { name: "Chạy ngay" })).toBeDisabled();
    expect(startThread).not.toHaveBeenCalled();
  });

  it("allows the same mixed selection in Riêng lẻ, which has no parent to find", async () => {
    render(<InteractionPopup devices={devices} selected={[]} onClose={() => undefined} />);
    await pasteLink();
    // One from each group, so the selection is genuinely mixed.
    fireEvent.click(screen.getByLabelText("Phone B"));
    fireEvent.click(screen.getByLabelText("Phone C"));
    // One control, three choices — the two dependent dropdowns are gone.
    fireEvent.click(screen.getByRole("radio", { name: /Riêng lẻ/ }));

    await clickRun();
    await waitFor(() => expect(startThread).toHaveBeenCalledTimes(1));
    const request = (startThread.mock.calls as unknown as Array<[ThreadCampaignRequest]>)[0][0];
    expect(request.actorUdids).toEqual(["actor-a", "actor-android"]);
    expect(request.mode).toBe("standalone");
    expect(request.shape).toBeUndefined();
  });

  it("blocks a one-actor run before dispatch rather than after it", async () => {
    // This used to dispatch, fail, and write the reason into a shared error string. The
    // check has not moved to the server — it has moved *earlier*, onto the button.
    render(<InteractionPopup devices={devices} selected={[]} onClose={() => undefined} />);
    await pasteLink();
    fireEvent.click(screen.getByLabelText("Phone B"));
    await waitFor(() => expect(screen.getByText(/Chọn từ 2 đến 64 máy/)).toBeVisible());
    expect(screen.getByRole("button", { name: "Chạy ngay" })).toBeDisabled();
    expect(startThread).not.toHaveBeenCalled();
  });

  it("sends the star shape by default, and the chain only when asked", async () => {
    // "Một máy bình luận gốc rồi các máy còn lại vào rep" — a star, and now the default:
    // a chain runs strictly one after another and one broken link stops the rest.
    render(<InteractionPopup devices={devices} selected={[]} onClose={() => undefined} />);
    await pasteLink();
    await clickRun();
    await waitFor(() => expect(startThread).toHaveBeenCalledTimes(1));
    let request = (startThread.mock.calls as unknown as Array<[ThreadCampaignRequest]>)[0][0];
    expect(request.mode).toBe("threaded");
    expect(request.shape).toBe("star");

    // The first run navigates to Theo dõi, which is the point of it — come back.
    fireEvent.click(screen.getByRole("tab", { name: "Thiết lập" }));
    fireEvent.click(screen.getByRole("radio", { name: /Nối tiếp/ }));
    await clickRun();
    await waitFor(() => expect(startThread).toHaveBeenCalledTimes(2));
    request = (startThread.mock.calls as unknown as Array<[ThreadCampaignRequest]>)[1][0];
    expect(request.shape).toBe("chain");
  });

  it("shows the teams the backend planned, not a copy of the split", async () => {
    // The popup used to reimplement `partition_actors` in TypeScript — remainder-spreading
    // and all — purely to draw this. Two implementations of one split are two chances to
    // show a plan that is not the plan.
    render(<InteractionPopup devices={devices} selected={[]} onClose={() => undefined} />);
    await pasteLink();
    await waitFor(() => expect(previewThread).toHaveBeenCalled());
    expect(await screen.findByText(/Cụm 1 · 2 máy/)).toBeVisible();
  });

  it("warns when more teams are planned than the app can stream at once", async () => {
    // `CapacityExhausted` is a refusal, not a queue: the cohorts past the limit fail rather
    // than wait, and before this the operator learned that from the Monitor tab.
    previewThread.mockResolvedValueOnce({
      lines: [],
      validTargetCount: 1,
      cohortCount: 6,
      streamCapacity: 2,
      plan: {
        requestId: "r",
        assignments: [
          {
            targetKey: "content:123",
            ordinal: 0,
            actorUdid: "actor-a",
            parentOrdinal: null,
            cohort: 0,
          },
        ],
      },
    } as never);
    render(<InteractionPopup devices={devices} selected={[]} onClose={() => undefined} />);
    await pasteLink();
    expect(await screen.findByText(/chỉ mở được 2 luồng màn hình/)).toBeVisible();
  });

  it("keeps the manual pool rule it has always advertised", async () => {
    // The hint said "cần ≥ N" and nothing enforced it, so the campaign row was written and
    // the backend refused it afterwards.
    render(<InteractionPopup devices={devices} selected={[]} onClose={() => undefined} />);
    await pasteLink();
    fireEvent.change(screen.getByLabelText(/Nội dung bình luận/), {
      target: { value: "manual" },
    });
    fireEvent.change(screen.getByLabelText(/Danh sách bình luận/), {
      target: { value: "đẹp quá" },
    });
    await waitFor(() =>
      expect(screen.getByText(/đang có 1 câu, cần ≥ 2/)).toBeVisible(),
    );
    expect(screen.getByRole("button", { name: "Chạy ngay" })).toBeDisabled();

    fireEvent.change(screen.getByLabelText(/Danh sách bình luận/), {
      target: { value: "đẹp quá\nchỗ này ở đâu ạ" },
    });
    await clickRun();
    await waitFor(() => expect(startThread).toHaveBeenCalledTimes(1));
    const request = (startThread.mock.calls as unknown as Array<[ThreadCampaignRequest]>)[0][0];
    expect(request.manualComments).toEqual(["đẹp quá", "chỗ này ở đâu ạ"]);
  });

  it("holds Chạy ngay while the plan is being recomputed for a new selection", async () => {
    // No IPC reordering needed. `largestCohort` is read out of the last preview, so between
    // changing the selection and the preview answering, the panel holds a number for a fleet
    // that is no longer selected. Deselect a phone, wait for the plan, reselect it, press
    // inside the 350 ms debounce, and the request went out with a message count for thirteen
    // actors and fourteen in the list — refused by the planner it was supposed to have asked.
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
    await pasteLink();
    await waitFor(() => expect(screen.getByRole("button", { name: "Chạy ngay" })).toBeEnabled());

    // Deselect one and let the plan settle on eight, which is the state that makes the stale
    // number wrong in the dangerous direction.
    fireEvent.click(screen.getByLabelText("Android 8"));
    await waitFor(() => expect(screen.getByRole("button", { name: "Chạy ngay" })).toBeEnabled());

    // Now put it back. Nine actors are selected and the plan on screen still says eight, so a
    // press here used to dispatch eight messages for nine actors — `TooFewMessagesForActors`,
    // from the planner the panel had just been talking to.
    fireEvent.click(screen.getByLabelText("Android 8"));
    expect(screen.getByRole("button", { name: "Chạy ngay" })).toBeDisabled();
    expect(screen.getByText(/Đang tính lại kế hoạch/)).toBeVisible();
    expect(startThread).not.toHaveBeenCalled();

    // And it lets go on its own once the plan catches up, with the count the fleet needs.
    await clickRun();
    await waitFor(() => expect(startThread).toHaveBeenCalledTimes(1));
    const request = (startThread.mock.calls as unknown as Array<[ThreadCampaignRequest]>)[0][0];
    expect(request.actorUdids).toHaveLength(9);
    expect(request.messageCount).toBe(9);
  });

  it("does not make the operator know the cohort size before pasting the comments", async () => {
    // Nine phones in teams of three needs three comments, and three is what is pasted — a
    // legal configuration the panel made unusable. The only channel that knows the cohort size
    // is the preview, and the preview was refused for a comment count computed from the cohort
    // size it was being asked to supply: the screen demanded "cần ≥ 9", a number that was never
    // true, with no way out but typing 3 by hand.
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
    await pasteLink();

    openAdvanced();
    fireEvent.change(screen.getByLabelText("Số máy mỗi cụm"), { target: { value: "3" } });
    fireEvent.change(screen.getByLabelText(/Nội dung bình luận/), {
      target: { value: "manual" },
    });
    fireEvent.change(screen.getByLabelText(/Danh sách bình luận/), {
      target: { value: "đẹp quá\nchỗ này ở đâu ạ\nxin info với" },
    });

    await clickRun();
    await waitFor(() => expect(startThread).toHaveBeenCalledTimes(1));
    const request = (startThread.mock.calls as unknown as Array<[ThreadCampaignRequest]>)[0][0];
    expect(request.cohortSize).toBe(3);
    expect(request.messageCount).toBe(3);
    expect(request.manualComments).toHaveLength(3);
    // The run carries the comments; the preview never did — that is what broke the loop.
    const previews = previewThread.mock.calls as unknown as Array<[ThreadCampaignRequest]>;
    expect(previews.at(-1)?.[0].manualComments).toEqual([]);
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
    await pasteLink();

    openAdvanced();
    fireEvent.change(screen.getByLabelText("Số máy mỗi cụm"), { target: { value: "3" } });
    fireEvent.change(screen.getByLabelText(/Số bình luận mỗi link/), {
      target: { value: "3" },
    });
    // The plan is re-asked for on a debounce, and until it answers the popup is still holding
    // the previous split — one team of nine, which three messages would not cover. It blocks
    // rather than guesses, so wait for the new plan the way the operator would.
    await clickRun();

    await waitFor(() => expect(startThread).toHaveBeenCalledTimes(1));
    const request = (startThread.mock.calls as unknown as Array<[ThreadCampaignRequest]>)[0][0];
    expect(request.actorUdids).toHaveLength(9);
    expect(request.cohortSize).toBe(3);
    expect(request.messageCount).toBe(3);
  });

  it("lets a whole-fleet run start without the operator doing the arithmetic", async () => {
    // `messageCount >= largest cohort` is a backend rule, and the old literal default of 2
    // against a pre-selected fleet meant the form opened already invalid. Auto follows the
    // plan instead.
    const fleet = Array.from({ length: 14 }, (_, index) => ({
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
    await pasteLink();
    await clickRun();
    await waitFor(() => expect(startThread).toHaveBeenCalledTimes(1));
    const request = (startThread.mock.calls as unknown as Array<[ThreadCampaignRequest]>)[0][0];
    expect(request.actorUdids).toHaveLength(14);
    expect(request.messageCount).toBe(14);
  });

  it("offers the number that would fix a too-small message count", async () => {
    render(<InteractionPopup devices={devices} selected={[]} onClose={() => undefined} />);
    await pasteLink();
    openAdvanced();
    fireEvent.change(screen.getByLabelText(/Số bình luận mỗi link/), {
      target: { value: "2" },
    });
    fireEvent.click(screen.getByLabelText("Phone C"));
    fireEvent.click(screen.getByRole("radio", { name: /Riêng lẻ/ }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Đặt = 3" })).toBeVisible());
    fireEvent.click(screen.getByRole("button", { name: "Đặt = 3" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Chạy ngay" })).toBeEnabled());
  });
});
