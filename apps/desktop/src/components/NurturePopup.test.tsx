import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { NurturePopup } from "./NurturePopup";
import { validateNurtureSettings } from "../nurtureValidation";
import { requestWorkspaceLeave, hasWorkspaceDrafts } from "../workspaceDraft";
import { requestSaveChanges } from "../confirmStore";
import type { DeviceInfo, DeviceMeta, NurtureSessionStatus, NurtureSettings } from "../types";

/**
 * Render tests for the redesigned panel, and they exist because a screenshot could not
 * prove it. The panel's body scrolls, so the lower half of the "Hành vi" tab — the human
 * rhythm switches and the bundle field — is off-screen in any capture, and the driver has
 * no scroll. A render assertion sees the whole tree, and unlike a screenshot it keeps
 * holding.
 */

/**
 * A note on the label queries below.
 *
 * Several captions are followed by an accessible `!` explanation button. Queries for the
 * actual input therefore specify its element type where the help button deliberately shares
 * the subject words. The help control has its own name and is exercised independently below.
 */
const saved = vi.hoisted(() => ({ saveSettings: vi.fn() }));
const profileControl = vi.hoisted(() => ({ render: vi.fn() }));

vi.mock("./AutomationProfileControl", () => ({
  AutomationProfileControl: (props: unknown) => {
    profileControl.render(props);
    return <div data-testid="nurture-profile-control" />;
  },
}));

vi.mock("../confirmStore", async (importOriginal) => ({
  ...await importOriginal<typeof import("../confirmStore")>(),
  requestSaveChanges: vi.fn(async () => "save"),
}));

/** The per-device ring, faked. Hoisted so the `../api` factory below can close over it. */
const logBook = vi.hoisted(() => ({
  read: vi.fn(),
  summary: vi.fn(async () => []),
  clear: vi.fn(),
}));

const settings: NurtureSettings = {
  baseUrl: "https://openrouter.ai/api/v1",
  model: "openai/gpt-5.6-luna",
  apiKey: "",
  bundleId: "com.ss.iphone.ugc.Ame",
  numVideos: 120,
  numRounds: 1,
  likeProb: 35,
  saveProb: 0,
  commentProb: 0,
  followProb: 3,
  frenzyProb: 6,
  watchMin: 3,
  watchMax: 18,
  persona: "casual",
  fatigue: true,
  timeOfDay: true,
  pauseSwipe: true,
  nightStart: 0,
  nightEnd: 0,
  recoverDelayMin: 2,
  recoverDelayMax: 4,
  staggerDelayMin: 5,
  staggerDelayMax: 15,
  commentLang: "vi",
  aiDirections: "Tự nhiên",
  maxCommentWords: 12,
  scheduleEnabled: false,
  scheduleEveryMinutes: 240,
  scheduleDurationMinutes: 150,
  scheduleUdids: [],
  likeEnabled: true,
  saveEnabled: false,
  commentEnabled: true,
  followEnabled: true,
  frenzyEnabled: true,
  carouselEnabled: true,
  carouselMaxSlides: 12,
  carouselPortionPercent: 100,
};

// The whole point of the Test API fix: the frames come from the WebView's decoder, not
// from the host's JPEG hub. `burst` is what the popup is expected to call, and the frames
// it returns are what must reach `nurtureTestApi`.
const burst = vi.fn(async () => [new Uint8Array([0xff, 0xd8, 0xff, 0x01])]);
vi.mock("../viewStore", () => ({
  exportViewJpegBurst: (...args: unknown[]) => burst(...(args as [])),
}));

vi.mock("../api", () => ({
  nurtureGetSettings: vi.fn(async () => settings),
  nurtureSaveSettings: saved.saveSettings,
  nurtureSessionStatus: vi.fn(async () => []),
  nurtureSessionLog: logBook.read,
  nurtureSessionLogSummary: logBook.summary,
  nurtureClearSessionLog: logBook.clear,
  nurtureStart: vi.fn(async () => undefined),
  nurtureStop: vi.fn(async () => undefined),
  nurtureTestApi: vi.fn(async () => null),
  nurtureListCommentAttempts: vi.fn(async () => []),
  nurtureCostSummary: vi.fn(async () => ({
    todayComments: 0,
    todayPromptTokens: 0,
    todayCompletionTokens: 0,
    totalComments: 0,
    totalPromptTokens: 0,
    totalCompletionTokens: 0,
  })),
  listenRiviuEvents: vi.fn(async () => () => undefined),
}));

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const entry = (text: string, repeats = 1, at = "2026-08-23T07:22:07.000Z", lastAt = at) => ({
  at,
  lastAt,
  text,
  repeats,
});

/** Every field at its zero, so a test names only what it is about. */
const blankStatus: NurtureSessionStatus = {
  udid: "fixture",
  running: false,
  videosDone: 0,
  swipeAttempts: 0,
  likeAttempts: 0,
  saveAttempts: 0,
  saveNoops: 0,
  saveUncertain: 0,
  commentAttempts: 0,
  followAttempts: 0,
  likes: 0,
  saves: 0,
  comments: 0,
  follows: 0,
  lastMessage: "",
  sessionPromptTokens: 0,
    sessionCompletionTokens: 0,
  runId: null,
  runSize: 0,
  phase: "queued",
  outcome: null,
  videoTarget: 0,
  startedAt: null,
  deadlineAt: null,
  cleanupState: "pending",
  cleanupProof: null,
  cleanupError: null,
};

/** Opens the panel with one device already reporting a status, so a row exists to click.
 *
 * `runId` and `videoTarget` are set because the run bar and the per-device bar only render
 * for a row that belongs to a run and has a denominator — a row without them is the
 * idle-sweep case, which deliberately draws no bar. */
async function openWithRow(
  running = true,
  over: Partial<NurtureSessionStatus> = {},
  showLog = true,
) {
  const api = await import("../api");
  vi.mocked(api.nurtureSessionStatus).mockResolvedValueOnce([
    {
      udid: "mock-1",
      running,
      videosDone: 3,
      swipeAttempts: 4,
      likeAttempts: 2,
      commentAttempts: 0,
      followAttempts: 0,
      likes: 2,
      comments: 0,
      follows: 0,
      lastMessage: "feed đã lên",
      sessionPromptTokens: 0,
    sessionCompletionTokens: 0,
      runId: "run-1",
      runSize: 1,
      phase: running ? "watching" : "finished",
      outcome: running ? null : "done",
      videoTarget: 12,
      startedAt: new Date(Date.now() - 60_000).toISOString(),
      deadlineAt: new Date(Date.now() + 3_600_000).toISOString(),
      cleanupState: running ? "pending" : "processAbsent",
      cleanupProof: running
        ? null
        : { bundleId: "com.ss.iphone.ugc.Ame", oldPid: 741 },
      cleanupError: null,
      ...over,
    },
  ]);
  await open();
  if (!showLog) return;
  fireEvent.click(screen.getByRole("tab", { name: "Log" }));
  return waitFor(() =>
    expect(screen.getByRole("button", { name: /Máy 1 · iPhone Mock 01/ })).toBeVisible(),
  );
}

const devices: DeviceInfo[] = [
  {
    udid: "mock-1",
    name: "iPhone Mock 01",
    model: "iPhone10,1",
    platform: "ios",
    osVersion: "16.7.15",
    connection: "mock",
    status: "ready",
    wdaReady: true,
  },
];

function open(metas: Map<string, DeviceMeta> = new Map()) {
  render(
    <NurturePopup
      devices={devices}
      selected={[]}
      metas={metas}
      onClose={() => undefined}
    />,
  );
  return waitFor(() => expect(screen.getByRole("tab", { name: "Hành vi" })).toBeVisible());
}

/** Opens the panel with the four interaction rates set to these values, rest unchanged. */
async function openWithRates(like: number, comment: number, follow: number, frenzy: number) {
  const api = await import("../api");
  vi.mocked(api.nurtureGetSettings).mockResolvedValueOnce({
    ...settings,
    likeProb: like,
    commentProb: comment,
    followProb: follow,
    frenzyProb: frenzy,
  });
  await open();
}

const slider = (name: string) => screen.getByLabelText(`${name} thanh kéo phần trăm`);
const box = (name: string) => screen.getByLabelText(`${name} phần trăm`);

describe("NurturePopup", () => {
  it("unsubscribes when the async event subscription resolves after unmount", async () => {
    const api = await import("../api");
    let resolveListen!: (unlisten: () => void) => void;
    const unlisten = vi.fn();
    vi.mocked(api.listenRiviuEvents).mockImplementationOnce(
      () => new Promise((resolve) => { resolveListen = resolve; }),
    );
    const view = render(
      <NurturePopup devices={devices} selected={[]} metas={new Map()} surface="page" />,
    );
    await waitFor(() => expect(api.listenRiviuEvents).toHaveBeenCalledTimes(1));

    view.unmount();
    resolveListen(unlisten);

    await waitFor(() => expect(unlisten).toHaveBeenCalledTimes(1));
  });

  it("shows a typed bootstrap error and retries without remounting the workspace", async () => {
    const api = await import("../api");
    vi.mocked(api.nurtureGetSettings)
      .mockRejectedValueOnce(new Error("settings database unavailable"))
      .mockResolvedValue(settings);

    render(
      <NurturePopup
        devices={devices}
        selected={[]}
        metas={new Map()}
        surface="page"
      />,
    );

    expect(await screen.findByRole("alert")).toHaveTextContent("settings database unavailable");
    expect(screen.queryByRole("tab", { name: "Thiết lập" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Thử tải lại Nuôi TikTok" }));

    expect(await screen.findByRole("tab", { name: "Thiết lập" })).toBeVisible();
    expect(api.nurtureGetSettings).toHaveBeenCalledTimes(2);
  });

  it("renders as an embedded workspace without popup chrome", async () => {
    render(
      <NurturePopup
        devices={devices}
        selected={[]}
        metas={new Map()}
        surface="page"
      />,
    );

    const workspace = screen.getByRole("region", { name: "Không gian Nuôi TikTok" });
    expect(workspace).toHaveClass("nurture-workspace");
    expect(workspace.querySelector("[style*='translate']")).toBeNull();
    expect(screen.queryByRole("button", { name: "Đóng" })).toBeNull();
    await waitFor(() => expect(screen.getByRole("tab", { name: "Thiết lập" })).toBeVisible());
    expect(screen.queryByRole("list", { name: "Quy trình Nuôi TikTok" })).toBeNull();
  });

  it("offers a target-bound Nurture profile only on page setup", async () => {
    const targetRef = { type: "group", groupId: "group-a" } as const;
    render(
      <NurturePopup
        devices={devices}
        selected={[]}
        targetUdids={["mock-1"]}
        targetRef={targetRef}
        metas={new Map()}
        surface="page"
      />,
    );

    await screen.findByTestId("nurture-profile-control");
    const props = profileControl.render.mock.calls.at(-1)?.[0];
    expect(props).toMatchObject({
      kind: "nurture",
      target: targetRef,
      defaultName: "Hồ sơ Nuôi TikTok",
      config: {
        schemaVersion: 1,
        durationMinutes: settings.scheduleDurationMinutes,
        settings: { saveEnabled: false, saveProb: 0 },
      },
    });
    expect(JSON.stringify(props)).not.toContain("apiKey");

    fireEvent.click(screen.getByRole("tab", { name: "Theo dõi" }));
    expect(screen.getByTestId("nurture-profile-control")).not.toBeVisible();
  });

  it("keeps profile-only rates separate while saving edited credentials through the settings API", async () => {
    const api = await import("../api");
    const scope = { type: "explicit", udids: ["mock-1"] } as const;
    render(<NurturePopup devices={devices} selected={[]} targetRef={{ ...scope, udids: [...scope.udids] }} targetUdids={["mock-1"]} metas={new Map()} surface="page" />);
    await screen.findByTestId("nurture-profile-control");
    let props = profileControl.render.mock.calls.at(-1)?.[0];
    await act(async () => props.onApply({ revision: { targetRef: scope, config: { schemaVersion: 1, settings: { likeProb: 99 } } } }));
    fireEvent.click(screen.getByRole("tab", { name: "AI" }));
    const key = document.querySelector<HTMLInputElement>('[data-nurture-field="apiKey"]')!;
    fireEvent.change(key, { target: { value: "new-fixture-key" } });
    props = profileControl.render.mock.calls.at(-1)?.[0];
    await act(async () => props.onSaved());
    expect(hasWorkspaceDrafts()).toBe(true);
    vi.mocked(api.nurtureGetSettings).mockResolvedValueOnce(settings);
    saved.saveSettings.mockResolvedValueOnce({ ...settings, apiKey: "__riviu_keep_stored_key__", hasApiKey: true });
    vi.mocked(requestSaveChanges).mockResolvedValueOnce("save");
    await act(async () => { await requestWorkspaceLeave(["nurture-credentials"]); });
    expect(saved.saveSettings).toHaveBeenCalledWith(expect.objectContaining({ apiKey: "new-fixture-key", likeProb: settings.likeProb }));
    expect(api.nurtureStart).not.toHaveBeenCalled();
    expect(key).toHaveValue("__riviu_keep_stored_key__");
  });

  it("keeps automation profiles out of the legacy popup surface", async () => {
    render(
      <NurturePopup
        devices={devices}
        selected={[]}
        targetRef={{ type: "all" }}
        metas={new Map()}
        onClose={() => undefined}
      />,
    );

    await screen.findByRole("tab", { name: "Hành vi" });
    expect(screen.queryByTestId("nurture-profile-control")).toBeNull();
  });

  it("separates page settings from monitoring while keeping all three settings groups", async () => {
    const api = await import("../api");
    vi.mocked(api.nurtureSessionStatus).mockResolvedValueOnce([
      { ...blankStatus, udid: "mock-1", running: true, lastMessage: "feed đã lên" },
    ]);
    render(
      <NurturePopup
        devices={devices}
        selected={[]}
        metas={new Map()}
        surface="page"
      />,
    );

    const workspace = screen.getByRole("region", { name: "Không gian Nuôi TikTok" });
    const modes = await within(workspace).findByRole("tablist", {
      name: "Chế độ Nuôi TikTok",
    });
    expect(within(modes).getAllByRole("tab").map((tab) => tab.textContent)).toEqual([
      "Thiết lập",
      "Theo dõi",
    ]);
    expect(within(modes).getByRole("tab", { name: "Thiết lập" })).toHaveAttribute(
      "aria-selected",
      "true",
    );

    const settingsTabs = within(workspace).getByRole("tablist", {
      name: "Nhóm thiết lập Nuôi TikTok",
    });
    expect(within(settingsTabs).getAllByRole("tab").map((tab) => tab.textContent)).toEqual([
      "Hành vi",
      "AI",
      "Bình luận",
    ]);
    expect(screen.queryByText("feed đã lên")).toBeNull();

    fireEvent.click(within(modes).getByRole("tab", { name: "Theo dõi" }));
    expect(within(modes).getByRole("tab", { name: "Theo dõi" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    const monitor = within(workspace).getByRole("tabpanel", { name: "Theo dõi" });
    expect(monitor).toBeVisible();
    expect(monitor).toHaveAttribute("aria-labelledby", "nurture-page-tab-monitor");
    expect(within(workspace).queryByRole("tab", { name: "Hành vi" })).toBeNull();
    expect(screen.getByText("feed đã lên")).toBeVisible();

    fireEvent.click(within(modes).getByRole("tab", { name: "Thiết lập" }));
    expect(within(workspace).getByRole("tab", { name: "Hành vi" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.queryByText("feed đã lên")).toBeNull();
  });

  it("provides complete keyboard and panel semantics for both Nurture tab levels", async () => {
    render(
      <NurturePopup
        devices={devices}
        selected={[]}
        metas={new Map()}
        surface="page"
      />,
    );

    const workspace = screen.getByRole("region", { name: "Không gian Nuôi TikTok" });
    const modes = await within(workspace).findByRole("tablist", {
      name: "Chế độ Nuôi TikTok",
    });
    const setup = within(modes).getByRole("tab", { name: "Thiết lập" });
    const monitor = within(modes).getByRole("tab", { name: "Theo dõi" });
    expect(setup).toHaveAttribute("tabindex", "0");
    expect(monitor).toHaveAttribute("tabindex", "-1");
    for (const mode of [setup, monitor]) {
      expect(document.getElementById(mode.getAttribute("aria-controls")!)).toHaveAttribute(
        "role",
        "tabpanel",
      );
    }

    setup.focus();
    fireEvent.keyDown(setup, { key: "ArrowRight" });
    expect(monitor).toHaveFocus();
    expect(monitor).toHaveAttribute("aria-selected", "true");
    const monitorPanel = document.getElementById(monitor.getAttribute("aria-controls")!);
    expect(monitorPanel).toHaveAttribute("role", "tabpanel");
    expect(monitorPanel).toHaveAttribute("aria-labelledby", monitor.id);

    fireEvent.keyDown(monitor, { key: "Home" });
    expect(setup).toHaveFocus();
    expect(setup).toHaveAttribute("aria-selected", "true");
    fireEvent.keyDown(setup, { key: "End" });
    expect(monitor).toHaveFocus();
    fireEvent.keyDown(monitor, { key: "ArrowLeft" });
    expect(setup).toHaveFocus();

    const settingsTabs = within(workspace).getByRole("tablist", {
      name: "Nhóm thiết lập Nuôi TikTok",
    });
    const behaviour = within(settingsTabs).getByRole("tab", { name: "Hành vi" });
    const ai = within(settingsTabs).getByRole("tab", { name: "AI" });
    const comments = within(settingsTabs).getByRole("tab", { name: "Bình luận" });
    expect(behaviour).toHaveAttribute("tabindex", "0");
    expect(ai).toHaveAttribute("tabindex", "-1");
    expect(comments).toHaveAttribute("tabindex", "-1");
    for (const settingsTab of [behaviour, ai, comments]) {
      expect(document.getElementById(settingsTab.getAttribute("aria-controls")!)).toHaveAttribute(
        "aria-labelledby",
        settingsTab.id,
      );
    }

    behaviour.focus();
    fireEvent.keyDown(behaviour, { key: "ArrowRight" });
    expect(ai).toHaveFocus();
    expect(ai).toHaveAttribute("aria-selected", "true");
    fireEvent.keyDown(ai, { key: "End" });
    expect(comments).toHaveFocus();
    fireEvent.keyDown(comments, { key: "ArrowRight" });
    expect(behaviour).toHaveFocus();
    fireEvent.keyDown(behaviour, { key: "ArrowLeft" });
    expect(comments).toHaveFocus();
    fireEvent.keyDown(comments, { key: "Home" });
    expect(behaviour).toHaveFocus();
  });

  it("opens page monitoring after a nurture run starts", async () => {
    const api = await import("../api");
    saved.saveSettings.mockResolvedValueOnce(settings);
    render(
      <NurturePopup
        devices={devices}
        selected={[]}
        metas={new Map()}
        surface="page"
      />,
    );

    const monitor = await screen.findByRole("tab", { name: "Theo dõi" });
    expect(monitor).toHaveAttribute("aria-selected", "false");
    fireEvent.click(screen.getByRole("button", { name: "Bắt đầu" }));

    await waitFor(() => expect(api.nurtureStart).toHaveBeenCalledWith(["mock-1"]));
    await waitFor(() => expect(monitor).toHaveAttribute("aria-selected", "true"));
    expect(screen.getByRole("tabpanel", { name: "Theo dõi" })).toBeVisible();
  });

  it("keeps Start disabled when the resolved page target group is empty", async () => {
    const api = await import("../api");
    render(
      <NurturePopup
        devices={devices}
        selected={[]}
        targetUdids={[]}
        metas={new Map()}
        surface="page"
      />,
    );

    const start = await screen.findByRole("button", { name: "Bắt đầu" });
    expect(start).toBeDisabled();
    fireEvent.click(start);
    expect(api.nurtureStart).not.toHaveBeenCalled();
  });

  it("blocks invalid settings before Start and profile save, and focuses the field to repair", async () => {
    const api = await import("../api");
    vi.mocked(api.nurtureGetSettings).mockResolvedValueOnce({ ...settings, watchMax: 1 });
    render(
      <NurturePopup devices={devices} selected={[]} metas={new Map()} surface="page"
        targetRef={{ type: "all" }} />,
    );

    const start = await screen.findByRole("button", { name: "Bắt đầu" });
    expect(start).toBeDisabled();
    const review = screen.getByRole("complementary", { name: "Kiểm tra trước khi chạy" });
    expect(within(review).queryByText("Sẵn sàng")).toBeNull();
    expect(within(review).getByText("Cần sửa thiết lập")).toBeVisible();
    expect(profileControl.render).toHaveBeenLastCalledWith(expect.objectContaining({ disabled: true }));
    fireEvent.click(start);
    expect(api.nurtureStart).not.toHaveBeenCalled();
    expect(api.nurtureSaveSettings).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Sửa thiết lập" }));
    const input = screen.getByLabelText(/Xem max/, { selector: "input" });
    expect(input).toHaveFocus();
    expect(input).toHaveAttribute("aria-invalid", "true");
    expect(input).toHaveAccessibleDescription(/Thời gian xem tối đa/);
    fireEvent.change(input, { target: { value: "18" } });
    expect(start).toBeEnabled();
    expect(input).not.toHaveAttribute("aria-invalid");
    expect(within(review).getByText("Sẵn sàng")).toBeVisible();
    expect(screen.queryByRole("button", { name: "Sửa thiết lập" })).toBeNull();
    expect(profileControl.render).toHaveBeenLastCalledWith(expect.objectContaining({ disabled: false }));
  });

  it("takes a missing comment key to AI and re-enables Start after repair", async () => {
    const api = await import("../api");
    vi.mocked(api.nurtureGetSettings).mockResolvedValueOnce({ ...settings, commentProb: 20 });
    render(<NurturePopup devices={devices} selected={[]} metas={new Map()} surface="page" />);
    const start = await screen.findByRole("button", { name: "Bắt đầu" });
    expect(start).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Sửa thiết lập" }));
    expect(screen.getByRole("tab", { name: "AI" })).toHaveAttribute("aria-selected", "true");
    const key = screen.getByLabelText(/Khóa API/, { selector: "input" });
    expect(key).toHaveFocus();
    expect(key).toHaveAccessibleDescription(/điền API key/);
    fireEvent.change(key, { target: { value: "fixture-key" } });
    expect(start).toBeEnabled();
    expect(api.nurtureStart).not.toHaveBeenCalled();
  });

  it.each(["scheduleEveryMinutes", "scheduleDurationMinutes"] as const)(
    "reveals and focuses invalid %s even with an existing schedule window", async (field) => {
      const api = await import("../api");
      vi.mocked(api.nurtureGetSettings).mockResolvedValueOnce({
        ...settings, [field]: 1,
        scheduleWindows: [{ id: "w-1", startMinute: 480, endMinute: 600,
          everyMinutes: 60, durationMinutes: 20, udids: [], behaviour: null }],
      });
      render(<NurturePopup devices={devices} selected={[]} metas={new Map()} surface="page" />);
      const start = await screen.findByRole("button", { name: "Bắt đầu" });
      expect(start).toBeDisabled();
      fireEvent.click(screen.getByRole("button", { name: "Sửa thiết lập" }));
      const input = document.querySelector(`[data-nurture-field="${field}"]`);
      expect(input).toHaveFocus();
      expect(input).toHaveAttribute("aria-invalid", "true");
      fireEvent.change(input!, { target: { value: "60" } });
      expect(start).toBeEnabled();
      expect(screen.queryByRole("button", { name: "Sửa thiết lập" })).toBeNull();
      expect(api.nurtureStart).not.toHaveBeenCalled();
    },
  );

  it("stops the devices captured by Start even after the page target changes", async () => {
    const api = await import("../api");
    const second = { ...devices[0], udid: "mock-2", name: "iPhone Mock 02" };
    vi.mocked(api.nurtureSessionStatus).mockResolvedValue([
      { ...blankStatus, udid: "mock-2", running: true },
    ]);
    saved.saveSettings.mockResolvedValue(settings);
    const { rerender } = render(
      <NurturePopup
        devices={[devices[0], second]}
        selected={[]}
        targetUdids={["mock-1"]}
        metas={new Map()}
        surface="page"
      />,
    );

    await screen.findByRole("button", { name: "Bắt đầu" });
    fireEvent.click(screen.getByRole("button", { name: "Bắt đầu" }));
    await waitFor(() => expect(api.nurtureStart).toHaveBeenCalledWith(["mock-1"]));

    rerender(
      <NurturePopup
        devices={[devices[0], second]}
        selected={[]}
        targetUdids={["mock-2"]}
        metas={new Map()}
        surface="page"
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Dừng" }));

    await waitFor(() => expect(api.nurtureStop).toHaveBeenCalledTimes(1));
    expect(api.nurtureStop).toHaveBeenCalledWith(["mock-1"]);
  });

  it("keeps Stop available from the Start snapshot while live status is still empty", async () => {
    const api = await import("../api");
    vi.mocked(api.nurtureSessionStatus).mockResolvedValue([]);
    saved.saveSettings.mockResolvedValue(settings);
    render(
      <NurturePopup
        devices={devices}
        selected={[]}
        targetUdids={["mock-1"]}
        metas={new Map()}
        surface="page"
      />,
    );

    const start = await screen.findByRole("button", { name: "Bắt đầu" });
    expect(screen.getByRole("button", { name: "Dừng" })).toBeDisabled();
    fireEvent.click(start);
    await waitFor(() => expect(api.nurtureStart).toHaveBeenCalledWith(["mock-1"]));
    const stop = screen.getByRole("button", { name: "Dừng" });
    expect(stop).toBeEnabled();
    fireEvent.click(stop);
    await waitFor(() => expect(api.nurtureStop).toHaveBeenCalledWith(["mock-1"]));
  });

  it("puts the device log in its own tab beside the three settings tabs", async () => {
    await openWithRow(true, {}, false);

    const tabs = screen.getAllByRole("tab").map((tab) => tab.textContent);
    expect(tabs).toEqual(["Hành vi", "AI", "Bình luận", "Log"]);
    expect(screen.queryByText("feed đã lên")).toBeNull();

    fireEvent.click(screen.getByRole("tab", { name: "Log" }));
    expect(screen.getByText("feed đã lên")).toBeVisible();
    expect(screen.queryByText("đang chạy · Lưu để áp ngay")).toBeNull();
  });

  it.each([
    ["save", "Đang lưu"],
    ["save skip: state unreadable", "Bỏ lưu: không đọc được trạng thái"],
    ["save fail: audit unavailable", "Lưu lỗi: không ghi được nhật ký"],
    ["save uncertain: card changed", "Lưu chưa chắc chắn: thẻ đã đổi"],
  ])("localizes the Save status %s", async (raw, translated) => {
    await openWithRow(true, { lastMessage: raw });
    expect(screen.getByText(translated)).toBeVisible();
  });

  it("labels a log row with the operator's device number and name, never only the model", async () => {
    const api = await import("../api");
    vi.mocked(api.nurtureSessionStatus).mockResolvedValueOnce([
      { ...blankStatus, udid: "mock-1", lastMessage: "failed — ONE-01", outcome: "failed" },
    ]);
    const metas = new Map<string, DeviceMeta>([
      [
        "mock-1",
        {
          udid: "mock-1",
          notes: "",
          tags: [],
          alias: "ONE-01",
          number: 7,
        },
      ],
    ]);

    await open(metas);
    fireEvent.click(screen.getByRole("tab", { name: "Log" }));

    expect(screen.getByRole("button", { name: /Máy 7 · ONE-01/ })).toBeVisible();
    expect(screen.queryByText("iPhone10,1")).toBeNull();
  });

  it("groups the settings into tabs and shows one group at a time", async () => {
    await open();
    // The settings tabs keep "Hành vi" selected first; Log is the fourth operational view.
    //
    // The schedule used to be a fourth tab. It now lives at the bottom of Hành vi, because a
    // window overrides the rates in that pane and the two were a tab apart — so this asserts
    // the schedule is reachable *without* leaving the default pane.
    expect(screen.getByRole("tab", { name: "Hành vi" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tab", { name: "AI" })).toHaveAttribute("aria-selected", "false");
    expect(screen.queryByRole("tab", { name: "Lịch" })).toBeNull();
    expect(screen.getByLabelText(/Lịch tự chạy/, { selector: "input" })).toBeVisible();
    // The AI group is not merely collapsed, it is not rendered — which is the point of
    // tabs over the three stacked collapsibles this replaced.
    expect(screen.queryByLabelText(/^Địa chỉ API/)).toBeNull();

    fireEvent.click(screen.getByRole("tab", { name: "AI" }));
    expect(screen.getByLabelText(/^Địa chỉ API/)).toBeVisible();
    expect(screen.getByLabelText(/^Mô hình/)).toBeVisible();
    expect(screen.getByLabelText(/^Khóa API/)).toBeVisible();
    expect(screen.getByLabelText(/^Tối đa từ/)).toBeVisible();
    expect(screen.getByLabelText(/^Định hướng giọng điệu/)).toBeVisible();
    expect(screen.getByRole("button", { name: /Kiểm tra API/ })).toBeVisible();

    // Back to Hành vi: the schedule's own fields are the empty-window fallback, which is
    // still a real mode — no windows means one cadence, all day.
    fireEvent.click(screen.getByRole("tab", { name: "Hành vi" }));
    expect(screen.getByLabelText(/^Mỗi \(phút\)/)).toBeVisible();
    expect(screen.getByLabelText(/^Thời lượng \(phút\)/)).toBeVisible();
  });

  it("keeps dated model benchmarks and farm anecdotes out of the operator UI", async () => {
    await open();
    fireEvent.click(screen.getByRole("tab", { name: "AI" }));

    expect(screen.queryByText(/19\/08\/2026/)).toBeNull();
    expect(screen.queryByText(/14 comment gửi/)).toBeNull();
    expect(screen.queryByText(/max_tokens/)).toBeNull();
  });

  it("gives every feature its own switch, separate from its percentage", async () => {
    await open();
    // The switch and the number are two controls, not one: turning a feature off must not
    // destroy the tuned percentage.
    for (const name of ["Thích", "Bình luận", "Theo dõi", "Vuốt nhanh"]) {
      expect(screen.getByLabelText(`Bật ${name}`)).toBeChecked();
    }
    expect(screen.getByLabelText("Bật Lưu")).not.toBeChecked();
    const like = screen.getByLabelText("Bật Thích");
    fireEvent.click(like);
    expect(like).not.toBeChecked();
    // …and the 35 is still there, still editable.
    expect(screen.getByLabelText("Thích phần trăm")).toHaveValue(35);
    expect(screen.getByLabelText("Thích phần trăm")).toBeEnabled();
  });

  it("renders the carousel as one switched row with its portion", async () => {
    await open();
    expect(screen.getByLabelText("Bật vuốt ngang bài ảnh")).toBeChecked();
    // The ceiling is no longer a field: it is a safety bound in the engine, not a number an
    // operator has a reason to pick. 100% means "to the end of the post", and the traversal
    // learns where that is by watching a swipe stop changing the screen.
    expect(screen.getByLabelText("Xem bao nhiêu phần trăm bài ảnh")).toHaveValue(100);
    expect(screen.queryByLabelText("Trần số ảnh")).toBeNull();
  });

  it("exposes the human-rhythm features that the old panel never reached", async () => {
    // `fatigue`, `timeOfDay` and `pauseSwipe` have been in `NurtureSettings` from the
    // start and no version of this UI showed any of them, so an operator could not turn a
    // single one off. This is the assertion that they are reachable — and it covers the
    // part of the tab that scrolls out of any screenshot.
    await open();
    expect(screen.getByLabelText(/^Mỏi dần/)).toBeChecked();
    expect(screen.getByLabelText(/^Theo giờ trong ngày/)).toBeChecked();
    expect(screen.getByLabelText(/^Ngập ngừng khi vuốt/)).toBeChecked();
    expect(screen.getByLabelText(/^Nghỉ đêm từ/, { selector: "input" })).toHaveValue(0);
    expect(screen.getByLabelText("Nghỉ đêm đến", { selector: "input" })).toHaveValue(0);
    expect(screen.getByLabelText(/Bundle TikTok/, { selector: "input" })).toHaveValue("com.ss.iphone.ugc.Ame");
  });

  it("explains every control through accessible help instead of a wall of hint text", async () => {
    // The explanations used to be permanent paragraphs under the fields and were removed
    // for making a settings form read as documentation. They came back as one glyph per
    // control, so the assertion is that each control has one and that it says something
    // specific — an unlabeled icon with an empty or generic tooltip would be worse than none.
    await open();
    const info = (of: string) => {
      const el = document.querySelector<HTMLElement>(`[data-info="${of}"]`);
      expect(el, `no help control for ${of}`).not.toBeNull();
      return el!;
    };
    for (const name of [
      "Giới hạn video",
      "Vòng",
      "Thích",
      "Lưu",
      "Bình luận",
      "Theo dõi",
      "Vuốt nhanh",
      "Xem min",
      "Xem max",
      "Mỏi dần",
      "Theo giờ trong ngày",
      "Ngập ngừng khi vuốt",
      "Nghỉ đêm",
      "Vuốt ngang",
      "Bundle TikTok",
    ]) {
      expect(info(name)).toHaveAttribute("aria-label", `Giải thích ${name}`);
      expect(info(name).querySelector("svg[aria-hidden='true']")).not.toBeNull();
      // A generic tooltip would be worse than none.
      expect(info(name).getAttribute("data-tip")!.length).toBeGreaterThan(30);
    }
    // Help is operable, not decorative: screen-reader and keyboard users receive its explicit
    // name while the adjacent input retains its own label.
    expect(screen.getByRole("button", { name: "Giải thích Mỏi dần" })).toBe(info("Mỏi dần"));
    for (const name of ["Thích", "Lưu", "Bình luận", "Theo dõi", "Vuốt nhanh"]) {
      // The feature rows name their switch explicitly, so their names are provably
      // untouched by the glyph rather than merely matched loosely.
      expect(screen.getByLabelText(`Bật ${name}`)).toBeInTheDocument();
      expect(screen.getByLabelText(`${name} phần trăm`)).toBeInTheDocument();
    }

    fireEvent.click(screen.getByRole("tab", { name: "AI" }));
    for (const name of ["Địa chỉ API", "Mô hình", "Khóa API", "Ngôn ngữ", "Tối đa từ", "Định hướng giọng điệu"]) {
      expect(info(name)).toBeVisible();
    }
    expect(screen.getByLabelText(/^Địa chỉ API/)).toBeVisible();

    fireEvent.click(screen.getByRole("tab", { name: "Hành vi" }));
    for (const name of ["Lịch tự chạy", "Mỗi (phút)", "Thời lượng (phút)"]) {
      expect(info(name)).toBeVisible();
    }
    // The manual-run horizon is the one thing this field does *not* control, and that was
    // measured the hard way — so the tooltip has to say so.
    expect(info("Thời lượng (phút)").getAttribute("data-tip")).toContain("bấm tay");
  });

  it("shows the pacing override as one switch, off by default", async () => {
    // The operator's numbers are the real numbers unless this is on. `settings` in this
    // file has no `humanLimits` key at all, which is the stored-row case: absent reads as
    // off, the same as the Rust `#[serde(default)]`.
    await open();
    const pacing = screen.getByLabelText(/^Giới hạn nhịp người/);
    expect(pacing).not.toBeChecked();
    // The tooltip has to name what it would take back, in numbers — an operator who turns
    // this on is choosing a much slower run than the percentages above suggest.
    const why = document.querySelector('[data-info="Giới hạn nhịp người"]')!.getAttribute("data-tip")!;
    for (const fragment of ["8–16", "2 trong 5", "12–35"]) {
      expect(why).toContain(fragment);
    }

    fireEvent.click(pacing);
    expect(pacing).toBeChecked();
  });

  it("does not clutter the form with restart badges", async () => {
    await open();
    expect(screen.queryByText("cần chạy lại")).toBeNull();
  });

  it("tests the API against the frames the WebView already decoded", async () => {
    // Test API read only the host's JPEG hub. Android phones stopped publishing there when
    // the H.264 view path landed, so pressing this while watching a phone's live picture
    // answered "Chưa có frame stream cho thiết bị …" -- true about the hub and false about
    // the phone in front of the operator.
    const api = await import("../api");
    await open();
    fireEvent.click(screen.getByRole("tab", { name: "AI" }));
    fireEvent.click(screen.getByRole("button", { name: /Kiểm tra API/ }));

    await waitFor(() => expect(api.nurtureTestApi).toHaveBeenCalled());
    expect(burst).toHaveBeenCalledWith("mock-1");
    expect(vi.mocked(api.nurtureTestApi).mock.calls[0]).toEqual([
      "mock-1",
      [new Uint8Array([0xff, 0xd8, 0xff, 0x01])],
    ]);
  });
});

describe("Nurture readiness validation", () => {
  it.each([
    ["numVideos", 0], ["numVideos", 10_001], ["numVideos", NaN], ["numVideos", 1.5],
    ["numRounds", 101], ["numRounds", Infinity],
    ["watchMin", 0], ["watchMin", NaN], ["watchMax", 1], ["watchMax", 121],
    ["maxCommentWords", 31], ["maxCommentWords", NaN],
    ["scheduleEveryMinutes", 14], ["scheduleEveryMinutes", Infinity],
    ["scheduleDurationMinutes", 361], ["scheduleDurationMinutes", 15.5],
  ] as const)("rejects %s=%s before persistence", (field, value) => {
    expect(validateNurtureSettings({ ...settings, [field]: value })?.field).toBe(field);
  });

  it("preserves stored-key and disabled-comment semantics", () => {
    expect(validateNurtureSettings(settings)).toBeNull();
    expect(validateNurtureSettings({ ...settings, commentProb: 20, commentEnabled: false })).toBeNull();
    expect(validateNurtureSettings({ ...settings, commentProb: 20, apiKey: "__riviu_keep_stored_key__" })).toBeNull();
    expect(validateNurtureSettings({ ...settings, commentProb: 20, hasApiKey: true, apiKey: "" })?.field).toBe("apiKey");
  });
});

describe("independent nurture rates", () => {
  it("gives every public action and pacing rate its own complete 0..100 range", async () => {
    await open();
    for (const name of ["Thích", "Lưu", "Bình luận", "Theo dõi", "Vuốt nhanh"]) {
      expect(slider(name)).toHaveAttribute("min", "0");
      expect(slider(name)).toHaveAttribute("max", "100");
      expect(slider(name)).toHaveAttribute("data-ceiling", "100");
    }
    expect(screen.queryByText(/Còn .*\/ 100%/)).toBeNull();
  });

  it("lets all public actions and frenzy be 100 without changing neighbours", async () => {
    await open();
    for (const name of ["Thích", "Lưu", "Bình luận", "Theo dõi", "Vuốt nhanh"]) {
      fireEvent.change(slider(name), { target: { value: "100" } });
      expect(box(name)).toHaveValue(100);
    }
    expect(screen.queryByRole("alert")).toBeNull();
    expect(box("Thích")).toHaveValue(100);
    expect(box("Lưu")).toHaveValue(100);
    expect(box("Bình luận")).toHaveValue(100);
    expect(box("Theo dõi")).toHaveValue(100);
    expect(box("Vuốt nhanh")).toHaveValue(100);
  });

  it("keeps a switched-off rate editable and preserves its tuned number", async () => {
    await open();
    const saveSwitch = screen.getByLabelText("Bật Lưu");
    expect(saveSwitch).not.toBeChecked();
    fireEvent.change(slider("Lưu"), { target: { value: "73" } });
    expect(box("Lưu")).toHaveValue(73);
    expect(saveSwitch).not.toBeChecked();
  });

  it("does not demand an API key for comments it will never post", async () => {
    // `settings.apiKey` is "" in this fixture. With the switch off the run cannot comment, so
    // refusing the save over a missing key was refusing it over a feature that will not run.
    const api = await import("../api");
    await openWithRates(30, 20, 0, 0);
    fireEvent.click(screen.getByRole("button", { name: "Lưu" }));
    await waitFor(() =>
      expect(screen.getByText(/điền API key trong Cấu hình AI/)).toBeVisible(),
    );
    expect(api.nurtureSaveSettings).not.toHaveBeenCalled();

    fireEvent.click(screen.getByLabelText("Bật Bình luận"));
    fireEvent.click(screen.getByRole("button", { name: "Lưu" }));
    await waitFor(() => expect(api.nurtureSaveSettings).toHaveBeenCalled());
    // The 20 is still on the wire; only the switch says not to use it.
    expect(vi.mocked(api.nurtureSaveSettings).mock.calls[0][0]).toMatchObject({
      commentProb: 20,
      commentEnabled: false,
    });
  });

  it("renders every typed Save outcome counter for each device", async () => {
    await openWithRow(true, {
      saveAttempts: 3,
      saves: 2,
      saveNoops: 4,
      saveUncertain: 1,
    });
    const metrics = document.querySelector<HTMLElement>(".nurture-metrics");
    expect(metrics).not.toBeNull();
    expect(metrics).toHaveAttribute(
      "title",
      "đã xác nhận / đã thử — video · tim · lưu · bình luận · theo dõi",
    );
    expect(metrics!.textContent).toContain("2/3L");
    expect(
      screen.getByText("Lưu: 2 xác nhận · 3 lần chạm · 4 bỏ qua · 1 chưa chắc chắn"),
    ).toBeVisible();
  });

  /**
   * The panel used to show one overwritten sentence per phone and nothing else, so the
   * question "what did this one do before it got stuck" had no answer anywhere in the app.
   */
  it("opens one device's own history from its row", async () => {
    logBook.read.mockResolvedValue([
      entry("mở TikTok"),
      entry("bỏ qua trang mời kết bạn của TikTok"),
      entry("feed đã lên"),
    ]);
    await openWithRow();

    // Closed to begin with: the history is fetched when asked for, not for every row.
    expect(logBook.read).not.toHaveBeenCalled();
    const row = screen.getByRole("button", { name: /iPhone Mock 01/ });
    expect(row).toHaveAttribute("aria-expanded", "false");

    fireEvent.click(row);
    await waitFor(() => expect(screen.getByText("mở TikTok")).toBeVisible());
    expect(logBook.read).toHaveBeenCalledWith("mock-1");
    expect(screen.getByText("bỏ qua trang mời kết bạn của TikTok")).toBeVisible();
    expect(row).toHaveAttribute("aria-expanded", "true");

    fireEvent.click(row);
    await waitFor(() => expect(screen.queryByText("mở TikTok")).toBeNull());
  });

  /**
   * A repeated line arrives collapsed with a count — that is what keeps two hundred slots
   * from being one message. The count has to be on screen, or the ring reads as a log that
   * silently dropped everything.
   */
  it("shows a repeated line once, with how many times and for how long", async () => {
    logBook.read.mockResolvedValue([
      entry(
        "TikTok đang khởi động — chờ feed lên",
        48,
        "2026-08-23T07:22:07.000Z",
        "2026-08-23T07:24:07.000Z",
      ),
    ]);
    await openWithRow();
    fireEvent.click(screen.getByRole("button", { name: /iPhone Mock 01/ }));

    await waitFor(() =>
      expect(screen.getByText("TikTok đang khởi động — chờ feed lên")).toBeVisible(),
    );
    // Two minutes between the first and the latest repeat, so the span is said in minutes
    // rather than leaving the operator to multiply 48 by a poll interval they cannot see.
    expect(screen.getByText(/×48 · 2 phút/)).toBeVisible();
  });

  it("says nothing has been recorded rather than showing an empty box", async () => {
    logBook.read.mockResolvedValue([]);
    await openWithRow(false);
    fireEvent.click(screen.getByRole("button", { name: /iPhone Mock 01/ }));
    await waitFor(() => expect(screen.getByText("máy này chưa nói gì")).toBeVisible());
  });

  it("clears one device's history on request and re-reads it", async () => {
    logBook.read.mockResolvedValue([entry("mở TikTok")]);
    logBook.clear.mockResolvedValue(undefined);
    await openWithRow();
    fireEvent.click(screen.getByRole("button", { name: /iPhone Mock 01/ }));
    await waitFor(() => expect(screen.getByText("mở TikTok")).toBeVisible());

    logBook.read.mockResolvedValue([]);
    fireEvent.click(screen.getByRole("button", { name: "Xoá" }));
    await waitFor(() => expect(logBook.clear).toHaveBeenCalledWith("mock-1"));
    // The empty state is worded for a *running* phone here, because this one is running —
    // "chưa nói gì" would be wrong about a session that is mid-flight.
    await waitFor(() =>
      expect(screen.getByText("chưa có dòng nào — phiên vừa bắt đầu")).toBeVisible(),
    );
    expect(screen.queryByText("mở TikTok")).toBeNull();
  });

  /**
   * The idle sweep clears TikTok's onboarding pages off phones nobody is driving, and it
   * writes into the same book — but it produces no session and no status. Before rows
   * came from the log summary too, such a phone had a full history and nowhere to open it
   * from, which made the sweep's work invisible in the one panel built to show it.
   */
  it("gives a row to a phone the idle sweep wrote for, even with no session", async () => {
    const api = await import("../api");
    vi.mocked(api.nurtureSessionStatus).mockResolvedValueOnce([]);
    logBook.summary.mockResolvedValueOnce([
      {
        udid: "mock-1",
        lines: 2,
        last: entry("bỏ qua trang mời kết bạn của TikTok"),
      },
    ] as never);
    logBook.read.mockResolvedValue([
      entry("bỏ qua trang mời kết bạn của TikTok"),
      entry("đã đưa máy về feed"),
    ]);
    await open();
    fireEvent.click(screen.getByRole("tab", { name: "Log" }));

    const row = await screen.findByRole("button", { name: /iPhone Mock 01/ });
    // No counters: it never ran a session, and "0/0v" would read as a run that did nothing.
    expect(screen.getByText("tự khôi phục")).toBeVisible();
    expect(screen.getByText("bỏ qua trang mời kết bạn của TikTok")).toBeVisible();

    fireEvent.click(row);
    await waitFor(() => expect(screen.getByText("đã đưa máy về feed")).toBeVisible());
  });
  /**
   * The operator's own request: a bar while it runs, one total, and per-device detail on
   * click. The total is the one that has to be right — it divides by the run's own size, so
   * a phone that failed before reporting still occupies its slot.
   */
  it("shows a run-wide bar with the counts beside it", async () => {
    const api = await import("../api");
    vi.mocked(api.nurtureSessionStatus).mockResolvedValueOnce([
      {
        ...blankStatus,
        udid: "mock-1",
        running: true,
        phase: "watching",
        videosDone: 6,
        videoTarget: 12,
        runId: "run-1",
        runSize: 2,
        startedAt: new Date(Date.now() - 60_000).toISOString(),
        deadlineAt: new Date(Date.now() + 3_600_000).toISOString(),
      },
      {
        ...blankStatus,
        udid: "mock-2",
        running: false,
        phase: "finished",
        outcome: "failed",
        lastMessage: "failed — không mở được phiên điều khiển",
        videoTarget: 12,
        runId: "run-1",
        runSize: 2,
      },
    ]);
    await open();
    fireEvent.click(screen.getByRole("tab", { name: "Log" }));

    const bar = await screen.findByRole("progressbar", {
      name: "Tiến trình cả lượt chạy",
    });
    // One of two phones is settled (the failed one counts as a settled slot), and the other
    // is half way: (1 + 0.5) / 2 = 75%.
    expect(bar).toHaveAttribute("aria-valuenow", "75");
    expect(screen.getByText(/1\/2 máy · 75%/)).toBeVisible();
    // The chip is what stops a nearly-full bar reading as success.
    expect(screen.getByText("✕ 1 lỗi")).toBeVisible();
    expect(screen.getByText("● 1 đang chạy")).toBeVisible();
  });

  it("gives each running device its own bar, labelled by the bound that governs it", async () => {
    await openWithRow(true, { videosDone: 3, videoTarget: 12 });
    const bar = await screen.findByRole("progressbar", {
      name: "Tiến trình Máy 1 · iPhone Mock 01",
    });
    expect(bar).toHaveAttribute("aria-valuenow", "25");
    // The video count is ahead of one minute out of an hour, so the label names videos.
    expect(screen.getByText("3/12 video")).toBeVisible();
  });

  /**
   * The reading a count-only bar gets wrong: this session is minutes from ending on the
   * clock while its video count sits at 25%.
   */
  it("follows the clock and says the time left when the clock is the closer bound", async () => {
    await openWithRow(true, {
      videosDone: 3,
      videoTarget: 12,
      startedAt: new Date(Date.now() - 110 * 60_000).toISOString(),
      deadlineAt: new Date(Date.now() + 10 * 60_000).toISOString(),
    });
    const bar = await screen.findByRole("progressbar", {
      name: "Tiến trình Máy 1 · iPhone Mock 01",
    });
    expect(Number(bar.getAttribute("aria-valuenow"))).toBeGreaterThan(80);
    expect(screen.getByText(/còn ~10 phút/)).toBeVisible();
  });

  /**
   * A failed phone and a finished one rendered as the same grey row until now, which is how
   * two dead phones went unnoticed on a fourteen-phone run.
   */
  it("marks a failed device apart from a finished one and ranks it above", async () => {
    const api = await import("../api");
    vi.mocked(api.nurtureSessionStatus).mockResolvedValueOnce([
      {
        ...blankStatus,
        udid: "mock-1",
        phase: "finished",
        outcome: "done",
        lastMessage: "done — 12/12 video",
        videoTarget: 12,
        runId: "run-1",
        runSize: 2,
      },
      {
        ...blankStatus,
        udid: "mock-2",
        phase: "finished",
        outcome: "failed",
        lastMessage: "failed — máy đang ở màn hình khoá",
        videoTarget: 12,
        runId: "run-1",
        runSize: 2,
      },
    ]);
    await open();
    fireEvent.click(screen.getByRole("tab", { name: "Log" }));

    await waitFor(() => expect(screen.getByText("✕ 1 lỗi")).toBeVisible());
    const rows = document.querySelectorAll(".nurture-float-log-row");
    expect(rows).toHaveLength(2);
    // Failure first, and visibly a failure rather than the same grey as the finished run.
    expect(rows[0].className).toContain("is-failed");
    expect(rows[1].className).toContain("is-done");
  });

  /**
   * The idle popup sweep writes rows for phones that never ran a session, so they have no
   * target and no deadline. A bar there would sit at 0% and read as a stalled run.
   */
  it("draws no device bar for a row the idle sweep created", async () => {
    const api = await import("../api");
    vi.mocked(api.nurtureSessionStatus).mockResolvedValueOnce([]);
    logBook.summary.mockResolvedValueOnce([
      { udid: "mock-1", lines: 1, last: entry("đã đưa máy về feed") },
    ] as never);
    await open();
    fireEvent.click(screen.getByRole("tab", { name: "Log" }));

    await waitFor(() => expect(screen.getByText("tự khôi phục")).toBeVisible());
    expect(screen.queryByRole("progressbar")).toBeNull();
  });

  it("shows a typed process-absence proof only in the selected device detail", async () => {
    const api = await import("../api");
    vi.mocked(api.nurtureSessionStatus).mockResolvedValueOnce([{
      ...blankStatus,
      udid: "mock-1",
      phase: "finished",
      outcome: "done",
      lastMessage: "done — 12/12 video",
      cleanupState: "processAbsent",
      cleanupProof: { bundleId: "com.ss.iphone.ugc.Ame", oldPid: 741 },
      cleanupError: null,
    }]);
    render(
      <NurturePopup devices={devices} selected={[]} metas={new Map()} surface="page" />,
    );

    fireEvent.click(await screen.findByRole("tab", { name: "Theo dõi" }));
    expect(screen.queryByText("TikTok đã tắt")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /Máy 1 · iPhone Mock 01/ }));

    expect(screen.getByText("TikTok đã tắt")).toBeVisible();
    const deviceDetails = screen.getByRole("group", { name: "Chi tiết kỹ thuật thiết bị" });
    const rawUdid = within(deviceDetails).getByText("mock-1");
    expect(rawUdid).not.toBeVisible();
    fireEvent.click(within(deviceDetails).getByText("Chi tiết thiết bị"));
    expect(rawUdid).toBeVisible();
    fireEvent.click(screen.getByText("Chứng cứ tiến trình"));
    expect(screen.getByText("com.ss.iphone.ugc.Ame")).toBeVisible();
    expect(screen.getByText("741")).toBeVisible();
  });

  it("fails closed when cleanup has no process-absence proof", async () => {
    const api = await import("../api");
    vi.mocked(api.nurtureSessionStatus).mockResolvedValueOnce([{
      ...blankStatus,
      udid: "mock-1",
      phase: "finished",
      outcome: "partial",
      lastMessage: "partial — lỗi dọn TikTok",
      cleanupState: "failed",
      cleanupProof: null,
      cleanupError: "không đọc được trạng thái tiến trình",
    }]);
    render(
      <NurturePopup devices={devices} selected={[]} metas={new Map()} surface="page" />,
    );

    fireEvent.click(await screen.findByRole("tab", { name: "Theo dõi" }));
    fireEvent.click(screen.getByRole("button", { name: /Máy 1 · iPhone Mock 01/ }));
    expect(screen.getByText("Chưa xác nhận được TikTok đã tắt")).toBeVisible();
    fireEvent.click(screen.getByText("Chi tiết lỗi"));
    expect(screen.getByRole("alert")).toHaveTextContent("không đọc được trạng thái tiến trình");
  });
});
