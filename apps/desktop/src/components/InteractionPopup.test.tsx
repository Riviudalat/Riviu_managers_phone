import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { InteractionPopup } from "./InteractionPopup";
import type {
  DeviceInfo,
  DeviceMeta,
  ThreadCampaignRequest,
  ThreadPlanAssignment,
  ThreadPreview,
  TikTokLinkLine,
} from "../types";

const { parseLinks, resolveLinks, startThread, previewThread, measurePost } = vi.hoisted(() => ({
  measurePost: vi.fn(async () => ({
    now: { views: 1353, likes: 22, comments: 26 },
    plan: {
      views: { shortfall: 147, ceiling: null, passes: 15, unreachable: null },
      likes: {
        shortfall: 28,
        ceiling: 2,
        passes: null,
        unreachable: "cần thêm 28 like nhưng chỉ có 2 máy chưa like bài này",
      },
      comments: null,
    },
    viewsRead: true,
  })),
  parseLinks: vi.fn(async (): Promise<TikTokLinkLine[]> => [
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
  resolveLinks: vi.fn(async (): Promise<TikTokLinkLine[]> => []),
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
  previewThread: vi.fn(async (request: ThreadCampaignRequest): Promise<ThreadPreview> => {
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
  automationArchive: vi.fn(async () => undefined),
  automationCreate: vi.fn(),
  automationList: vi.fn(async () => []),
  automationRevise: vi.fn(),
  interactionCancel: vi.fn(async () => undefined),
  interactionGet: vi.fn(async () => null),
  interactionList: vi.fn(async () => []),
  interactionParseLinks: parseLinks,
  interactionPreviewThread: previewThread,
  interactionMeasurePost: measurePost,
  interactionResolveLinks: resolveLinks,
  interactionStartThread: startThread,
  listenRiviuEvents: vi.fn(async () => () => undefined),
  listGroups: vi.fn(async () => []),
  // Reached on mount, once per in-scope device, to load each phone's @handle.
  getDeviceMeta: vi.fn(async (udid: string) => ({
    udid,
    notes: "",
    tags: [],
    groupId: null,
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

/**
 * No operator records at all, which is the state a fresh install is in: every phone keeps the
 * name it reports and gets its grid position as a number. Every case below uses this, so the
 * one case that *does* pass records is unmistakably about them.
 */
const noMeta = new Map<string, DeviceMeta>();

const devices: DeviceInfo[] = [
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
];

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((next) => {
    resolve = next;
  });
  return { promise, resolve };
}

function parsedLine(id: string): TikTokLinkLine {
  const url = `https://www.tiktok.com/@creator/video/${id}`;
  return {
    lineNo: 1,
    original: url,
    target: {
      originalUrl: url,
      normalizedUrl: url,
      targetKey: `content:${id}`,
      contentId: id,
      author: "creator",
      kind: "video",
    },
    error: null,
  };
}

function previewOf(request: ThreadCampaignRequest): ThreadPreview {
  return {
    lines: [],
    validTargetCount: request.targets.length,
    cohortCount: 1,
    streamCapacity: 8,
    plan: {
      requestId: request.requestId,
      assignments: request.actorUdids.map((actorUdid, ordinal) => ({
        targetKey: request.targets[0]?.targetKey ?? "content:123",
        ordinal,
        actorUdid,
        parentOrdinal: ordinal === 0 ? null : 0,
        cohort: 0,
      })),
    },
  };
}

it("shows target-bound automation profiles only on the dedicated page", async () => {
  const { rerender } = render(
    <InteractionPopup
      metas={noMeta}
      devices={devices}
      selected={[]}
      surface="page"
      targetRef={{ type: "group", groupId: "morning" }}
    />,
  );
  expect(await screen.findByRole("region", { name: "Quản lý hồ sơ Tương tác" })).toBeVisible();

  rerender(
    <InteractionPopup
      metas={noMeta}
      devices={devices}
      selected={[]}
      onClose={() => undefined}
    />,
  );
  expect(screen.queryByRole("region", { name: "Quản lý hồ sơ Tương tác" })).not.toBeInTheDocument();
});

it("renders the production action workflow and a live review rail on the page", async () => {
  render(
    <InteractionPopup
      metas={noMeta}
      devices={devices}
      selected={[]}
      surface="page"
      targetRef={{ type: "all" }}
    />,
  );

  const workflow = await screen.findByRole("list", { name: "Quy trình Tương tác" });
  expect(within(workflow).getAllByRole("listitem").map((item) => item.textContent)).toEqual([
    "1Phạm vi",
    "2Hành động",
    "3Kiểm tra",
    "4Theo dõi",
  ]);
  const review = screen.getByRole("complementary", { name: "Kiểm tra chiến dịch" });
  expect(within(review).getByText("Link hợp lệ")).toBeVisible();
  expect(within(review).getByText("Thiết bị chạy")).toBeVisible();
  expect(within(review).getByText("Hành động")).toBeVisible();
});

/**
 * Paste the one link the parse mock knows, and wait until it has actually been parsed.
 *
 * The wait is on the ✓ marker in the link list, not on the URL text: React mirrors a
 * textarea's value into its child text, so matching the URL matches what was just typed and
 * resolves before the (debounced) parse has returned anything.
 */
async function pasteLink() {
  fireEvent.change(screen.getByRole("textbox", { name: "Link TikTok — mỗi dòng một link" }), {
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
  it("does not let an old short-link resolution replace links parsed from newer input", async () => {
    const oldResolution = deferred<TikTokLinkLine[]>();
    const oldUrl = "https://vt.tiktok.com/old";
    const newUrl = "https://www.tiktok.com/@creator/video/222";
    parseLinks
      .mockResolvedValueOnce([
        { lineNo: 1, original: oldUrl, target: null, error: "unresolvedShortLink" },
      ])
      .mockResolvedValueOnce([parsedLine("222")]);
    resolveLinks.mockReturnValueOnce(oldResolution.promise);
    render(<InteractionPopup metas={noMeta} devices={devices} selected={[]} onClose={() => undefined} />);
    const input = screen.getByRole("textbox", { name: "Link TikTok — mỗi dòng một link" });

    fireEvent.change(input, { target: { value: oldUrl } });
    fireEvent.click(await screen.findByRole("button", { name: "Gỡ link rút gọn" }));
    fireEvent.change(input, { target: { value: newUrl } });
    await waitFor(() =>
      expect(document.querySelector(".interaction-link-list")?.textContent).toContain(newUrl),
    );

    await act(async () => {
      oldResolution.resolve([parsedLine("111")]);
      await oldResolution.promise;
    });
    expect(document.querySelector(".interaction-link-list")?.textContent).toContain(newUrl);
    expect(document.querySelector(".interaction-link-list")?.textContent).not.toContain("111");
  });

  it("keeps the newest preview and run gate when an older preview resolves last", async () => {
    const oldPreview = deferred<ThreadPreview>();
    previewThread
      .mockReturnValueOnce(oldPreview.promise)
      .mockImplementationOnce(async (request) => previewOf(request));
    const allHierarchy: DeviceInfo[] = devices.map((device) => ({
      ...device,
      platform: "android",
    }));
    render(
      <InteractionPopup
        metas={noMeta}
        devices={allHierarchy}
        selected={[]}
        onClose={() => undefined}
      />,
    );
    await pasteLink();
    await waitFor(() => expect(previewThread).toHaveBeenCalledTimes(1));

    fireEvent.click(screen.getByLabelText("Phone C"));
    await waitFor(() => expect(previewThread).toHaveBeenCalledTimes(2));
    expect(await screen.findByText(/Cụm 1 · 2 máy/)).toBeVisible();
    await waitFor(() => expect(screen.getByRole("button", { name: "Chạy ngay" })).toBeEnabled());

    const firstRequest = previewThread.mock.calls[0][0];
    await act(async () => {
      oldPreview.resolve(previewOf(firstRequest));
      await oldPreview.promise;
    });
    expect(screen.getByText(/Cụm 1 · 2 máy/)).toBeVisible();
    expect(screen.queryByText(/Cụm 1 · 3 máy/)).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Chạy ngay" })).toBeEnabled();
    expect(screen.queryByText(/Đang tính lại kế hoạch/)).not.toBeInTheDocument();
  });

  it("configures independent actions and runs a one-actor like/save campaign without comment fields", async () => {
    render(<InteractionPopup metas={noMeta} devices={devices} selected={[]} onClose={() => undefined} />);
    await pasteLink();

    fireEvent.click(screen.getByRole("checkbox", { name: /^Tim$/ }));
    fireEvent.click(screen.getByRole("checkbox", { name: /^Lưu$/ }));
    fireEvent.click(screen.getByRole("checkbox", { name: /^Bình luận$/ }));
    fireEvent.click(screen.getByLabelText("Phone B"));

    expect(screen.getByText("Thực hiện theo thứ tự: Tim → Lưu.")).toBeVisible();
    expect(screen.queryByRole("radiogroup", { name: "Kiểu tương tác" })).toBeNull();
    expect(screen.queryByLabelText(/Nội dung bình luận/)).toBeNull();
    expect(screen.queryByLabelText(/Hướng dẫn giọng điệu/)).toBeNull();
    expect(screen.queryByText(/Tag thêm acc/)).toBeNull();
    expect(screen.queryAllByPlaceholderText("@handle")).toHaveLength(0);
    expect(screen.getByLabelText("Phone A")).toBeChecked();

    await clickRun();
    await waitFor(() => expect(startThread).toHaveBeenCalledTimes(1));
    const request = (startThread.mock.calls as unknown as Array<[ThreadCampaignRequest]>)[0][0];
    expect(request.actorUdids).toEqual(["actor-a"]);
    expect(request.actions).toEqual({ like: true, comment: false, save: true });
    expect(request).not.toHaveProperty("likeTarget");
  });

  it("renders as an embedded workspace without popup chrome", () => {
    render(
      <InteractionPopup
        metas={noMeta}
        devices={devices}
        selected={[]}
        surface="page"
      />,
    );

    const workspace = screen.getByRole("region", { name: "Không gian Tương tác" });
    expect(workspace).toHaveClass("interaction-workspace");
    expect(workspace.querySelector("[style*='translate']")).toBeNull();
    expect(screen.queryByRole("button", { name: "Đóng" })).toBeNull();
    expect(screen.getByRole("tab", { name: "Thiết lập" })).toBeVisible();
    expect(screen.getByRole("tab", { name: "Theo dõi" })).toBeVisible();
  });

  it("moves and activates workspace tabs with the complete horizontal keyboard pattern", () => {
    render(
      <InteractionPopup
        metas={noMeta}
        devices={devices}
        selected={[]}
        surface="page"
      />,
    );

    const setup = screen.getByRole("tab", { name: "Thiết lập" });
    const monitor = screen.getByRole("tab", { name: "Theo dõi" });
    for (const tab of [setup, monitor]) {
      const panel = document.getElementById(tab.getAttribute("aria-controls")!);
      expect(panel).toHaveAttribute("role", "tabpanel");
      expect(panel).toHaveAttribute("aria-labelledby", tab.id);
    }
    setup.focus();
    fireEvent.keyDown(setup, { key: "ArrowRight" });
    expect(monitor).toHaveFocus();
    expect(monitor).toHaveAttribute("aria-selected", "true");

    fireEvent.keyDown(monitor, { key: "Home" });
    expect(setup).toHaveFocus();
    expect(setup).toHaveAttribute("aria-selected", "true");

    fireEvent.keyDown(setup, { key: "End" });
    expect(monitor).toHaveFocus();
    fireEvent.keyDown(monitor, { key: "ArrowLeft" });
    expect(setup).toHaveFocus();
  });

  it("parses multiline links and submits every selected actor", async () => {
    render(<InteractionPopup metas={noMeta} devices={devices} selected={[]} onClose={() => undefined} />);
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
    render(<InteractionPopup metas={noMeta} devices={devices} selected={[]} onClose={() => undefined} />);
    await pasteLink();
    await clickRun();
    await waitFor(() => expect(api.interactionGet).toHaveBeenCalledWith("campaign-1"));
  });

  it("offers Android as an actor, grouped by how it reads the screen", async () => {
    // Android used to be filtered out here because the Interaction send path was
    // pixel-only. It now drives the drawer through the accessibility hierarchy, so
    // excluding it would hide working hardware. What replaces the filter is the grouping
    // plus the mixed-thread refusal below.
    render(<InteractionPopup metas={noMeta} devices={devices} selected={[]} onClose={() => undefined} />);
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
  /// **A group means nothing in `Riêng lẻ`, so it is not offered there.**
  ///
  /// A group is a set of phones that talk to each other: Toả has them all reply to one root
  /// comment, Nối tiếp has them reply down the list in order. `Riêng lẻ` has no thread — every
  /// phone posts its own root comment and reads nobody — so a group there names a set with
  /// nothing to do with the shape, and a control that loads one invites the reading that it
  /// does something.
  it("offers the group loader for threaded shapes and hides it for Riêng lẻ", async () => {
    // The shared mock answers with no groups — the control only exists when one does, so
    // this case supplies one rather than changing what every other case sees.
    const api = await import("../api");
    vi.mocked(api.listGroups).mockResolvedValue([
      { id: "g1", name: "Nhóm 1", color: "#f97316", udids: ["actor-a"], createdAt: "2026-08-25T00:00:00Z" },
    ]);
    render(<InteractionPopup metas={noMeta} devices={devices} selected={[]} onClose={() => undefined} />);
    await waitFor(() => expect(screen.getByLabelText(/Lấy từ nhóm/)).toBeVisible());

    fireEvent.click(screen.getByRole("radio", { name: /Riêng lẻ/ }));
    await waitFor(() => expect(screen.queryByLabelText(/Lấy từ nhóm/)).toBeNull());

    fireEvent.click(screen.getByRole("radio", { name: /^Toả/ }));
    await waitFor(() => expect(screen.getByLabelText(/Lấy từ nhóm/)).toBeVisible());
  });

  /// **The number and the name are the operator's, and the model is gone.**
  ///
  /// This picker used to number phones by their index in the fleet list while a comment above
  /// it claimed the numbers matched the wall. They did not: the wall stamps
  /// `tileNumber(position, meta)`, so the first time anyone used Change Number the two
  /// disagreed and "máy số 7" meant a different phone in each place. On twenty identical
  /// SM-G950Fs the number is the only handle there is.
  ///
  /// The model came out for the same reason it was useless: every phone here reports the same
  /// one, so it filled the row the name needed. The udid stays as the tooltip identity.
  it("labels actors by the operator's number and name, not by model", async () => {
    const metas = new Map<string, DeviceMeta>([
      // Renamed and renumbered — deliberately out of fleet order, which is the case the old
      // index-based numbering got wrong.
      ["actor-android", { udid: "actor-android", notes: "", tags: [], alias: "Máy kho", number: 3 }],
      ["actor-a", { udid: "actor-a", notes: "", tags: [], number: 11 }],
    ]);
    render(
      <InteractionPopup metas={metas} devices={devices} selected={[]} onClose={() => undefined} />,
    );

    // The alias replaces the reported name wherever the phone is shown.
    expect(screen.getByLabelText("Máy kho")).toBeVisible();
    expect(screen.queryByLabelText("Phone C")).toBeNull();
    // The stored numbers are what is drawn, not 1..3 by position. Read off each tile rather
    // than off the panel: "3" also appears in the cohort and message fields.
    const numberOn = (label: string) =>
      screen
        .getByLabelText(label)
        .closest(".interaction-actor-tile")
        ?.querySelector(".tile-num")?.textContent;
    expect(numberOn("Máy kho")).toBe("3");
    expect(numberOn("Phone A")).toBe("11");
    expect(screen.getByRole("textbox", { name: "Tài khoản TikTok của Máy kho" })).toBeVisible();
    expect(screen.getByRole("textbox", { name: "Tài khoản TikTok của Phone A" })).toBeVisible();
    const technical = screen.getByRole("group", { name: "Chi tiết kỹ thuật Máy kho" });
    const rawId = within(technical).getByText("actor-android");
    expect(rawId).not.toBeVisible();
    fireEvent.click(within(technical).getByText("Chi tiết"));
    expect(rawId).toBeVisible();
    // And no model string anywhere in the panel.
    for (const model of ["iPhone 8", "Redmi Note 12"]) {
      expect(screen.queryByText(model)).toBeNull();
    }
  });


  it("keeps the operator's actor selection across a fleet poll", async () => {
    // The seeding effect depended on the actor lists, which are memos over `devices` -- a
    // fresh array every three seconds from the fleet poll. So the selection re-ran on every
    // tick and threw away whatever had just been chosen: selecting actors for a threaded
    // campaign was a race against the next poll, and nobody wins that.
    const { rerender } = render(
      <InteractionPopup metas={noMeta} devices={devices} selected={[]} onClose={() => undefined} />,
    );
    fireEvent.click(screen.getByLabelText("Phone B"));
    expect(screen.getByLabelText("Phone B")).not.toBeChecked();

    rerender(
      <InteractionPopup
        metas={noMeta}
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
      <InteractionPopup metas={noMeta} devices={devices} selected={[]} onClose={() => undefined} />,
    );
    expect(screen.getByLabelText("Phone A")).toBeChecked();
    expect(screen.getByLabelText("Phone B")).toBeChecked();

    rerender(
      <InteractionPopup
        metas={noMeta}
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
    render(<InteractionPopup metas={noMeta} devices={devices} selected={[]} onClose={() => undefined} />);
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
    render(<InteractionPopup metas={noMeta} devices={devices} selected={[]} onClose={() => undefined} />);
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
    render(<InteractionPopup metas={noMeta} devices={devices} selected={[]} onClose={() => undefined} />);
    await pasteLink();
    fireEvent.click(screen.getByLabelText("Phone B"));
    await waitFor(() => expect(screen.getByText(/Chọn từ 2 đến 64 máy/)).toBeVisible());
    expect(screen.getByRole("button", { name: "Chạy ngay" })).toBeDisabled();
    expect(startThread).not.toHaveBeenCalled();
  });

  it("sends the star shape by default, and the chain only when asked", async () => {
    // "Một máy bình luận gốc rồi các máy còn lại vào rep" — a star, and now the default:
    // a chain runs strictly one after another and one broken link stops the rest.
    render(<InteractionPopup metas={noMeta} devices={devices} selected={[]} onClose={() => undefined} />);
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
    render(<InteractionPopup metas={noMeta} devices={devices} selected={[]} onClose={() => undefined} />);
    await pasteLink();
    await waitFor(() => expect(previewThread).toHaveBeenCalled());
    expect(await screen.findByText(/Cụm 1 · 2 máy/)).toBeVisible();
  });

  it("warns when more teams are planned than the app can stream at once", async () => {
    // `CapacityExhausted` is a refusal, not a queue: the cohorts past the limit fail rather
    // than wait, and before this the operator learned that from the Monitor tab.
    //
    // Six links for six cohorts, so all six really do run. With fewer links than cohorts the
    // spare cohorts get no assignments at all and the warning would be about nothing — see the
    // test below.
    previewThread.mockResolvedValueOnce({
      lines: [],
      validTargetCount: 6,
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
    render(<InteractionPopup metas={noMeta} devices={devices} selected={[]} onClose={() => undefined} />);
    await pasteLink();
    expect(await screen.findByText(/chỉ mở được 2 luồng màn hình/)).toBeVisible();
  });

  it("does not warn about cohorts that have no link to work on", async () => {
    // `plan_threads` deals links round-robin and emits nothing for a cohort with no link, so
    // `cohortCount` is the partition and not the number of teams that run. Fourteen phones in
    // teams of three with **one** link gave `cohortCount = 4` against a capacity of 2, and the
    // panel warned that four cohorts would run and the excess be refused — directly above a
    // preview drawing exactly one, advising a change that would fix nothing.
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
    render(<InteractionPopup metas={noMeta} devices={devices} selected={[]} onClose={() => undefined} />);
    await pasteLink();
    await waitFor(() => expect(screen.getByText("Sẽ chạy như thế này")).toBeVisible());
    expect(screen.queryByText(/luồng màn hình/)).toBeNull();
  });

  it("gives the actors back when the phones come back", async () => {
    // The drop effect removed and the seeding effect never restored, so narrowing the wall
    // selection to one tile stripped the actor list — and widening it again did not undo that.
    // The operator had to re-tick every tile by hand, and a fleet poll that briefly returned a
    // short device list did the same thing.
    const { rerender } = render(
      <InteractionPopup
        metas={noMeta}
        devices={devices}
        selected={["actor-a", "actor-b"]}
        onClose={() => undefined}
      />,
    );
    await pasteLink();
    await waitFor(() => expect(screen.getByRole("button", { name: "Chạy ngay" })).toBeEnabled());

    // One tile on the wall, so one phone is in scope and two actors cannot be selected.
    rerender(
      <InteractionPopup metas={noMeta} devices={devices} selected={["actor-a"]} onClose={() => undefined} />,
    );
    await waitFor(() => expect(screen.getByText(/đang chọn 1/)).toBeVisible());

    // Back to both. The selection has to come back with them.
    rerender(
      <InteractionPopup
        metas={noMeta}
        devices={devices}
        selected={["actor-a", "actor-b"]}
        onClose={() => undefined}
      />,
    );
    await clickRun();
    await waitFor(() => expect(startThread).toHaveBeenCalledTimes(1));
    const request = (startThread.mock.calls as unknown as Array<[ThreadCampaignRequest]>)[0][0];
    expect(request.actorUdids).toEqual(["actor-a", "actor-b"]);
  });

  it("keeps the manual pool rule it has always advertised", async () => {
    // The hint said "cần ≥ N" and nothing enforced it, so the campaign row was written and
    // the backend refused it afterwards.
    render(<InteractionPopup metas={noMeta} devices={devices} selected={[]} onClose={() => undefined} />);
    await pasteLink();
    fireEvent.change(screen.getByLabelText(/Nội dung bình luận/), {
      target: { value: "manual" },
    });
    fireEvent.change(screen.getByLabelText(/Danh sách bình luận/, { selector: "textarea" }), {
      target: { value: "đẹp quá" },
    });
    await waitFor(() =>
      expect(screen.getByText(/1 câu · cần ≥ 2/)).toBeVisible(),
    );
    expect(screen.getByRole("button", { name: "Chạy ngay" })).toBeDisabled();

    fireEvent.change(screen.getByLabelText(/Danh sách bình luận/, { selector: "textarea" }), {
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
    render(<InteractionPopup metas={noMeta} devices={fleet} selected={[]} onClose={() => undefined} />);
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

  /// **The preview never carries the manual comments, and the run always does.**
  ///
  /// That split is what broke a loop the panel used to be stuck in: the preview was refused
  /// for a comment count it was itself being asked to supply, so the number on screen could
  /// never become true. The cohort field that made the loop reachable is gone now, but the
  /// split is what keeps the preview answerable at all, so it stays pinned.
  it("keeps the comment pool out of the preview and in the run", async () => {
    const fleet = Array.from({ length: 3 }, (_, index) => ({
      udid: `android-${index}`,
      name: `Android ${index}`,
      model: "SM-G955F",
      platform: "android",
      osVersion: "9",
      connection: "usb",
      status: "ready",
      wdaReady: true,
    })) as never[];
    render(<InteractionPopup metas={noMeta} devices={fleet} selected={[]} onClose={() => undefined} />);
    await pasteLink();

    fireEvent.change(screen.getByLabelText(/Nội dung bình luận/), {
      target: { value: "manual" },
    });
    fireEvent.change(screen.getByLabelText(/Danh sách bình luận/, { selector: "textarea" }), {
      target: { value: "đẹp quá\nchỗ này ở đâu ạ\nxin info với" },
    });

    await clickRun();
    await waitFor(() => expect(startThread).toHaveBeenCalledTimes(1));
    const request = (startThread.mock.calls as unknown as Array<[ThreadCampaignRequest]>)[0][0];
    // One cohort, always: the actor list is the cohort, so three phones want three comments.
    expect(request.cohortSize).toBeUndefined();
    expect(request.messageCount).toBe(3);
    expect(request.manualComments).toHaveLength(3);
    const previews = previewThread.mock.calls as unknown as Array<[ThreadCampaignRequest]>;
    expect(previews.at(-1)?.[0].manualComments).toEqual([]);
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
    render(<InteractionPopup metas={noMeta} devices={fleet} selected={[]} onClose={() => undefined} />);
    await pasteLink();
    await clickRun();
    await waitFor(() => expect(startThread).toHaveBeenCalledTimes(1));
    const request = (startThread.mock.calls as unknown as Array<[ThreadCampaignRequest]>)[0][0];
    expect(request.actorUdids).toHaveLength(14);
    expect(request.messageCount).toBe(14);
  });

  it("offers the number that would fix a too-small message count", async () => {
    render(<InteractionPopup metas={noMeta} devices={devices} selected={[]} onClose={() => undefined} />);
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

  it("reads the post on a press, and says what each target would take", async () => {
    // A view count is a navigation — TikTok states a play count only on the author's profile
    // grid, and the grid does not say which post a tile is, so each candidate is opened and its
    // caption compared. Timed 24/08/2026 that took about four and a half minutes for a post near
    // the top of the grid, longer when it sits deeper, and it holds a phone for all of it —
    // which is why nothing here runs on a paste or a debounce.
    render(<InteractionPopup metas={noMeta} devices={devices} selected={[]} onClose={() => undefined} />);
    await pasteLink();
    expect(measurePost).not.toHaveBeenCalled();

    fireEvent.change(screen.getByLabelText("View muốn đạt"), { target: { value: "1500" } });
    fireEvent.change(screen.getByLabelText("Tim muốn đạt"), { target: { value: "50" } });
    fireEvent.click(screen.getByLabelText("Đọc cả số view"));
    fireEvent.click(screen.getByRole("button", { name: "Đo bài" }));

    await waitFor(() => expect(measurePost).toHaveBeenCalledTimes(1));
    const call = measurePost.mock.calls[0] as unknown as [
      string,
      unknown,
      { views: number | null; likes: number | null; comments: number | null },
      number,
      boolean,
];

    // One phone answers a question about the post; the fleet size is what bounds a like target.
    expect(call[0]).toBe("actor-a");
    expect(call[2]).toEqual({ views: 1500, likes: 50, comments: null });
    expect(call[3]).toBe(2);
    expect(call[4]).toBe(true);

    // The shortfall and the pass estimate for what can be done…
    expect(await screen.findByText(/còn thiếu 147/)).toBeVisible();
    expect(screen.getByText(/ước 15 lượt/)).toBeVisible();
    // …and the refusal, in Vietnamese, for what cannot — before an hour of farming, not after.
    expect(screen.getByText(/chỉ có 2 máy chưa like/)).toBeVisible();
    // A metric with no target set is not a claim about the post.
    expect(screen.getByText(/Bình luận: đang 26 — không đặt ngưỡng/)).toBeVisible();
  });

  it("cannot read a post with no link to read", async () => {
    render(<InteractionPopup metas={noMeta} devices={devices} selected={[]} onClose={() => undefined} />);
    expect(screen.getByRole("button", { name: "Đo bài" })).toBeDisabled();
    await pasteLink();
    expect(screen.getByRole("button", { name: "Đo bài" })).toBeEnabled();
  });
});
