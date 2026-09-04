import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  automationCreate,
  listenRiviuEvents,
  publishGet,
  publishList,
  publishReadiness,
  publishReconcile,
  publishSheetGetConfig,
  publishSheetSaveConfig,
} from "../api";
import { PublishPage } from "./PublishPage";
import { requestConfirm } from "../confirmStore";
import { resetToasts } from "../toastStore";
import type {
  AppEvent,
  DeviceInfo,
  DeviceMeta,
  PublishBundle,
  PublishFolderManifest,
  PublishPreflightReport,
  PublishPreflightRequest,
} from "../types";

function bundle(id: string, name: string): PublishBundle {
  return {
    id,
    sourcePath: `C:/carousels/${name}`,
    name,
    mediaKind: "image",
    images: [],
    captionPath: `C:/carousels/${name}/caption.txt`,
    caption: `caption for ${name}`,
    captionSha256: `sha-${id}`,
    totalBytes: 1024,
  };
}

// Scanned-directory order. The operator sees this order in the folder and in the
// checkbox list; the question this file asks is whether it survives to the campaign.
const manifest: PublishFolderManifest = {
  sourceRoot: "C:/carousels",
  scannedAt: "2026-08-18T00:00:00.000Z",
  bundles: [bundle("b1", "bo1"), bundle("b2", "bo2"), bundle("b3", "bo3")],
  notices: [],
  ignoredPartnerFiles: 0,
  ignoredHiddenFiles: 0,
};

const createCampaign = vi.fn(async () => ({
  id: "campaign-1",
  state: "prepared",
  assignments: [],
  createdAt: "2026-08-18T00:00:00.000Z",
}));

const executeCampaign = vi.fn(async () => ({
  campaignId: "campaign-1",
  status: "complete",
  retryScope: "none",
  issues: [],
  detail: { campaign: { id: "campaign-1" }, bundles: [], assignments: [], events: [] },
}));

const preflightCampaign = vi.fn(async (request: PublishPreflightRequest): Promise<PublishPreflightReport> => ({
  inputDigest: "approved-digest-1",
  targetSnapshot: {
    targetRef: request.targetRef ?? { type: "explicit", udids: request.udids },
    included: request.udids.map((udid, index) => ({ udid, alias: `Máy ${index + 1}`, number: index + 1 })),
    excluded: [],
    rosterSha256: "11".repeat(32),
  },
  canExecute: true,
  assignments: request.bundleIds.map((bundleId, ordinal) => ({
    ordinal,
    bundleId,
    udid: request.udids[ordinal],
    packageName: "com.ss.android.ugc.trill",
    version: "38.3.2",
    locale: "en",
    media: "pass" as const,
    composer: "pass" as const,
    soundPicker: "pass" as const,
    storage: "pass" as const,
    requiredBytes: 1024,
    availableBytes: 4096,
    issues: [],
  })),
  issues: [],
  sheetConfigured: false,
}));

vi.mock("../pickFile", () => ({
  pickDirectory: vi.fn(async () => "C:/carousels"),
  pickIpa: vi.fn(async () => null),
  pickMaterial: vi.fn(async () => null),
}));

vi.mock("../confirmStore", () => ({
  requestConfirm: vi.fn(async () => true),
}));

vi.mock("../api", () => ({
  addAppLibrary: vi.fn(async () => undefined),
  addMaterial: vi.fn(async () => undefined),
  analyticsSummary: vi.fn(async () => ({})),
  automationArchive: vi.fn(async () => undefined),
  automationCreate: vi.fn(async () => ({
    definition: {
      id: "publish-profile-1",
      name: "Đăng bài theo thư mục",
      kind: "publish",
      latestRevision: 1,
      archived: false,
      createdAt: "2026-09-03T00:00:00Z",
      updatedAt: "2026-09-03T00:00:00Z",
    },
    revision: {
      definitionId: "publish-profile-1",
      revision: 1,
      target: { type: "group", groupId: "group-a" },
      config: {},
      canonicalJson: "{}",
      sha256: "aa".repeat(32),
      createdAt: "2026-09-03T00:00:00Z",
    },
  })),
  automationList: vi.fn(async () => []),
  automationRevise: vi.fn(),
  apiDocs: vi.fn(async () => ""),
  deleteAppLibrary: vi.fn(async () => undefined),
  deleteMaterial: vi.fn(async () => undefined),
  deleteSchedule: vi.fn(async () => undefined),
  exampleScript: vi.fn(async () => "{}"),
  installIpaToGroup: vi.fn(async () => []),
  installLibraryApp: vi.fn(async () => undefined),
  // The page follows a live run now, so it subscribes. Returning a no-op unsubscriber keeps
  // the effect's cleanup honest without the test caring about events.
  listenRiviuEvents: vi.fn(async () => () => undefined),
  publishAutoAssign: vi.fn(async () => ({ plan: [] })),
  listAppsLibrary: vi.fn(async () => []),
  listGroups: vi.fn(async () => []),
  listMaterials: vi.fn(async () => []),
  listSchedules: vi.fn(async () => []),
  listScripts: vi.fn(async () => []),
  publishCancel: vi.fn(async () => undefined),
  publishCreateCampaign: (...args: unknown[]) => createCampaign(...(args as [])),
  publishExecute: (...args: unknown[]) => executeCampaign(...(args as [])),
  publishGet: vi.fn(async () => null),
  publishList: vi.fn(async () => []),
  publishPreflight: (...args: unknown[]) => preflightCampaign(...(args as [PublishPreflightRequest])),
  publishReadiness: vi.fn(async () => []),
  publishReconcile: vi.fn(async (campaignId: string) => ({
    campaignId,
    inputDigest: "approved-digest-1",
    status: "partial",
    retryScope: "fullPipeline",
    reportJson: {},
    updatedAt: "2026-09-04T00:00:00Z",
  })),
  publishSheetGetConfig: vi.fn(async () => ({ webhookUrl: "", hasToken: false })),
  publishSheetSaveConfig: vi.fn(async () => ({ webhookUrl: "", hasToken: false })),
  publishScanFolder: vi.fn(async () => manifest),
  pushMaterial: vi.fn(async () => undefined),
  saveSchedule: vi.fn(async () => undefined),
  saveScript: vi.fn(async () => undefined),
}));

function iphone(udid: string): DeviceInfo {
  return {
    udid,
    name: udid,
    model: "iPhone10,1",
    platform: "ios",
    osVersion: "16.7.15",
    connection: "usb",
    status: "ready",
    wdaReady: true,
  };
}

const devices = [iphone("PHONE-A"), iphone("PHONE-B"), iphone("PHONE-C")];

beforeEach(() => {
  createCampaign.mockClear();
  executeCampaign.mockClear();
  preflightCampaign.mockClear();
  vi.mocked(publishReconcile).mockReset().mockImplementation(async (campaignId) => ({
    campaignId,
    inputDigest: "approved-digest-1",
    status: "partial",
    retryScope: "fullPipeline",
    reportJson: {},
    updatedAt: "2026-09-04T00:00:00Z",
  }));
  vi.mocked(requestConfirm).mockReset().mockResolvedValue(true);
  vi.mocked(publishSheetGetConfig)
    .mockReset()
    .mockResolvedValue({ webhookUrl: "", hasToken: false });
  vi.mocked(publishSheetSaveConfig)
    .mockReset()
    .mockResolvedValue({ webhookUrl: "", hasToken: false });
  resetToasts();
});

afterEach(cleanup);

describe("publish, bundle to phone", () => {
  it("distinguishes loading, load failure with retry, and a genuinely empty monitor", async () => {
    const user = userEvent.setup();
    const list = vi.mocked(publishList);
    let rejectFirst!: (reason: Error) => void;
    list.mockImplementationOnce(
      () => new Promise((_, reject) => {
        rejectFirst = reject;
      }),
    );

    render(<PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />);
    await user.click(screen.getByRole("tab", { name: "Theo dõi" }));
    expect(screen.getByText("Đang tải chiến dịch…")).toBeVisible();
    expect(screen.queryByText("Chưa có chiến dịch")).toBeNull();

    rejectFirst(new Error("không đọc được chiến dịch"));
    expect(await screen.findByRole("alert")).toHaveTextContent("không đọc được chiến dịch");
    expect(screen.queryByText("Chưa có chiến dịch")).toBeNull();

    list.mockResolvedValueOnce([] as never);
    await user.click(screen.getByRole("button", { name: "Thử lại" }));
    expect(await screen.findByText("Chưa có chiến dịch")).toBeVisible();
  });

  it("presents one publish workspace for photos and video", () => {
    render(<PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />);

    expect(screen.queryByRole("heading", { level: 1 })).toBeNull();
    expect(screen.getByText(/một video hoặc một bộ ảnh/i)).toBeInTheDocument();
    expect(screen.queryByText(/dùng âm thanh mặc định/i)).toBeNull();
  });

  it("provides a roving keyboard tablist linked to named publish panels", async () => {
    render(<PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />);

    const setup = screen.getByRole("tab", { name: "Thiết lập" });
    const monitor = screen.getByRole("tab", { name: "Theo dõi" });
    expect(setup).toHaveAttribute("tabindex", "0");
    expect(monitor).toHaveAttribute("tabindex", "-1");
    expect(document.getElementById(setup.getAttribute("aria-controls")!)).toHaveAttribute(
      "aria-label",
      "Thiết lập",
    );

    setup.focus();
    fireEvent.keyDown(setup, { key: "ArrowRight" });
    await waitFor(() => expect(monitor).toHaveFocus());
    expect(monitor).toHaveAttribute("aria-selected", "true");
    const monitorPanel = document.getElementById(monitor.getAttribute("aria-controls")!);
    expect(monitorPanel).toHaveAttribute("role", "tabpanel");
    expect(monitorPanel).toHaveAttribute("aria-label", "Theo dõi");

    fireEvent.keyDown(monitor, { key: "Home" });
    await waitFor(() => expect(setup).toHaveFocus());
    expect(setup).toHaveAttribute("aria-selected", "true");
    fireEvent.keyDown(setup, { key: "End" });
    await waitFor(() => expect(monitor).toHaveFocus());
    fireEvent.keyDown(monitor, { key: "ArrowLeft" });
    await waitFor(() => expect(setup).toHaveFocus());
  });

  it("separates setup from monitoring and saves a target-bound publish profile", async () => {
    const user = userEvent.setup();
    render(
      <PublishPage
        devices={[devices[0]]}
        selected={["PHONE-A"]}
        targetUdids={["PHONE-A"]}
        targetRef={{ type: "group", groupId: "group-a" }}
        onSelectUdids={() => {}}
      />,
    );

    expect(screen.getByRole("tab", { name: "Thiết lập" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await user.click(screen.getByRole("button", { name: "Chọn thư mục" }));
    await user.click(await screen.findByRole("button", { name: "Tạo hồ sơ" }));

    await waitFor(() => {
      expect(automationCreate).toHaveBeenCalledWith(
        "Đăng bài theo thư mục",
        "publish",
        { type: "group", groupId: "group-a" },
        expect.objectContaining({
          schemaVersion: 1,
          sourceRoot: "C:/carousels",
          bundleIds: ["b1"],
          executionConfirmed: true,
          soundPolicy: expect.objectContaining({ kind: "trendingAny", poolSize: 5 }),
        }),
      );
    });
    expect(requestConfirm).toHaveBeenCalledWith(expect.objectContaining({
      title: "Cho phép hồ sơ đăng công khai?",
      confirmLabel: "Cho phép và lưu",
    }));

    await user.click(screen.getByRole("tab", { name: "Theo dõi" }));
    expect(screen.getByText("Chưa có chiến dịch")).toBeVisible();
    expect(screen.queryByRole("button", { name: "Chọn thư mục" })).not.toBeInTheDocument();
  });

  it("snapshots an edited caption and starts the confirmed trending-sound pipeline", async () => {
    const user = userEvent.setup();
    render(
      <PublishPage
        devices={[devices[0]]}
        selected={["PHONE-A"]}
        onSelectUdids={() => {}}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Chọn thư mục" }));
    const caption = await screen.findByRole("textbox", { name: "Chú thích cho bo1" });
    await user.clear(caption);
    await user.type(caption, "Caption đã duyệt");
    await user.click(screen.getByRole("button", { name: "Chạy preflight" }));
    await screen.findByText(/Có thể chuyển sang xác nhận công khai/);
    await user.click(screen.getByRole("button", { name: /Xác nhận và đăng/ }));

    await waitFor(() => expect(createCampaign).toHaveBeenCalledTimes(1));
    expect(createCampaign.mock.calls[0]).toEqual([
      "C:/carousels",
      ["b1"],
      ["PHONE-A"],
      null,
      { b1: "Caption đã duyệt" },
      expect.objectContaining({ kind: "trendingAny", poolSize: 5, seed: expect.any(Number) }),
      { type: "all" },
      true,
      "approved-digest-1",
    ]);
    expect(executeCampaign).toHaveBeenCalledWith("campaign-1", true);
    expect(screen.queryByRole("button", { name: "Post" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Prepare" })).toBeNull();
    expect(screen.queryByRole("button", { name: /Transfer media/ })).toBeNull();
  });

  it("keeps public execution locked until current input passes preflight", async () => {
    const user = userEvent.setup();
    render(
      <PublishPage
        devices={[devices[0]]}
        selected={["PHONE-A"]}
        onSelectUdids={() => {}}
      />,
    );

    const submit = screen.getByRole("button", { name: /Xác nhận và đăng/ });
    expect(submit).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "Chọn thư mục" }));
    expect(submit).toBeDisabled();

    await user.click(screen.getByRole("button", { name: "Chạy preflight" }));
    await waitFor(() => expect(submit).toBeEnabled());
    expect(preflightCampaign).toHaveBeenCalledWith(expect.objectContaining({
      sourceRoot: "C:/carousels",
      bundleIds: ["b1"],
      udids: ["PHONE-A"],
      targetRef: { type: "all" },
      captionOverrides: { b1: "caption for bo1" },
      soundPolicy: expect.objectContaining({ kind: "trendingAny", poolSize: 5 }),
    }));

    await user.type(screen.getByRole("textbox", { name: "Chú thích cho bo1" }), " mới");
    expect(submit).toBeDisabled();
    expect(screen.getByText("Chưa có preflight hợp lệ")).toBeVisible();
    await user.click(submit);
    expect(createCampaign).not.toHaveBeenCalled();
  });

  it("shows a scoped preflight failure and leaves public execution locked", async () => {
    const user = userEvent.setup();
    preflightCampaign.mockResolvedValueOnce({
      inputDigest: "rejected-digest",
      targetSnapshot: {
        targetRef: { type: "explicit", udids: ["PHONE-A"] },
        included: [{ udid: "PHONE-A", alias: "Máy 1", number: 1 }],
        excluded: [],
        rosterSha256: "22".repeat(32),
      },
      canExecute: false,
      assignments: [{
        ordinal: 0,
        bundleId: "b1",
        udid: "PHONE-A",
        packageName: "com.ss.android.ugc.trill",
        version: "38.3.2",
        locale: "en",
        media: "pass",
        composer: "fail",
        soundPicker: "fail",
        storage: "pass",
        requiredBytes: 1024,
        availableBytes: 4096,
        issues: [],
      }],
      issues: [{ code: "sound_picker_unmeasured", message: "Bộ chọn nhạc chưa được đo trên build này." }],
      sheetConfigured: false,
    });
    render(<PublishPage devices={[devices[0]]} selected={["PHONE-A"]} onSelectUdids={() => {}} />);

    await user.click(screen.getByRole("button", { name: "Chọn thư mục" }));
    await user.click(screen.getByRole("button", { name: "Chạy preflight" }));

    expect(await screen.findByText("Bộ chọn nhạc chưa được đo trên build này.")).toBeVisible();
    expect(screen.getByRole("button", { name: /Xác nhận và đăng/ })).toBeDisabled();
    expect(createCampaign).not.toHaveBeenCalled();
  });

  /**
   * The pairing is positional all the way down — `validate_publish_mapping` zips
   * `bundle_ids[i]` with `udids[i]` — so the order of the array that is *sent* is the
   * whole contract. The screen shows one order and used to send another: the preview
   * iterated the bundles in scanned-folder order while the dispatch sent them in the
   * order the checkboxes happened to be clicked.
   *
   * Nothing errors when they disagree. Each phone is a different live TikTok account,
   * so what it costs is one account posting another's photographs under another's
   * caption, with no discrepancy to notice afterwards and no delete path to undo it.
   */
  it("sends the bundles in the order it showed them, whatever order they were ticked", async () => {
    const user = userEvent.setup();
    render(
      <PublishPage
        devices={devices}
        selected={["PHONE-A", "PHONE-B", "PHONE-C"]}
        onSelectUdids={() => {}}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Chọn thư mục" }));
    await waitFor(() => expect(screen.getByText(/1\. bo1/)).toBeTruthy());

    // Reconsider the middle bundle and put it back — the ordinary hesitation that made
    // the two arrays disagree. Ticking bottom-up does the same thing.
    const boxes = screen.getAllByRole("checkbox");
    await user.click(boxes[1]);
    await user.click(boxes[1]);

    // Read the pairing off the screen, exactly as the operator would.
    const shown = screen
      .getAllByText(/^\d+\. bo\d$/)
      .map((node) => node.textContent!.replace(/^\d+\.\s*/, ""));
    expect(shown).toEqual(["bo1", "bo2", "bo3"]);

    await user.click(screen.getByRole("button", { name: "Chạy preflight" }));
    await screen.findByText(/Có thể chuyển sang xác nhận công khai/);
    await user.click(screen.getByRole("button", { name: /Xác nhận và đăng/ }));
    await waitFor(() => expect(createCampaign).toHaveBeenCalled());

    const [, dispatchedIds, dispatchedTargets] = createCampaign.mock.calls[0] as unknown as [
      string,
      string[],
      string[],
    ];
    const byId = new Map(manifest.bundles.map((b) => [b.id, b.name]));
    expect(
      dispatchedIds.map((id) => byId.get(id)),
      "the campaign pairs bundles with phones in a different order than the screen promised",
    ).toEqual(shown);
    expect(dispatchedTargets).toEqual(["PHONE-A", "PHONE-B", "PHONE-C"]);
  });

  /**
   * **The last answer on screen has to be the newest one asked for.**
   *
   * A run emits `publishUpdated` several times in a few seconds — one per phone as its
   * assignment moves — and each one started its own `publishList()` with nothing sequencing
   * them. Two commands in flight over the same USB bus do not come back in the order they went
   * out, so the reload started while the campaign was `posting` could resolve *after* the one
   * started once it read `succeeded`, and put `posting` back on screen.
   *
   * That state then stays. Nothing else re-reads until the next event, and the last event of a
   * run is the one that says it finished — so the page an operator is watching ends the run
   * showing a campaign still working, on phones that are already idle.
   */
  it("keeps the newest reload when an older one answers late", async () => {
    const listen = vi.mocked(listenRiviuEvents);
    const list = vi.mocked(publishList);
    // The earlier test in this file mounted the page too, and the mock counts calls
    // across both. The sequencing question is about *which* call answers last, so the
    // count has to start from this render.
    list.mockReset();
    let fire: (event: AppEvent) => void = () => {};
    listen.mockImplementation(async (handler: (event: AppEvent) => void) => {
      fire = handler;
      return () => undefined;
    });

    const campaign = (state: string) => [
      {
        id: "campaign-1",
        requestId: "req-1",
        sourceRoot: "C:/carousels",
        state,
        runAt: null,
        visibility: "public",
        cleanupPolicy: "afterPost",
        assignments: [],
        createdAt: "2026-08-18T00:00:00.000Z",
        updatedAt: "2026-08-18T00:00:00.000Z",
        errorCode: null,
      },
    ];

    // Mount reads an empty list; then two events, and the answers come back swapped.
    let releasePosting = () => {};
    let releaseSucceeded = () => {};
    list.mockResolvedValueOnce([] as never);
    list.mockReturnValueOnce(
      new Promise((resolve) => {
        releasePosting = () => resolve(campaign("posting") as never);
      }) as never,
    );
    list.mockReturnValueOnce(
      new Promise((resolve) => {
        releaseSucceeded = () => resolve(campaign("succeeded") as never);
      }) as never,
    );

    render(<PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />);
    await waitFor(() => expect(list).toHaveBeenCalledTimes(1));

    fire({ type: "publishUpdated", campaignId: "campaign-1", revision: 2 });
    await waitFor(() => expect(list).toHaveBeenCalledTimes(2));
    fire({ type: "publishUpdated", campaignId: "campaign-1", revision: 3 });
    await waitFor(() => expect(list).toHaveBeenCalledTimes(3));

    releaseSucceeded();
    await waitFor(() => expect(screen.getByText("Hoàn tất")).toBeTruthy());
    releasePosting();

    // The late answer is discarded rather than rendered. Waiting first would pass even
    // without the guard, so this settles the microtask queue and then looks.
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(
      screen.queryByText("Đang đăng"),
      "a reload that started earlier repainted the page over a newer one",
    ).toBeNull();
    expect(screen.getByText("Hoàn tất")).toBeTruthy();

    list.mockReset();
    list.mockResolvedValue([] as never);
    listen.mockReset();
    listen.mockImplementation(async () => () => undefined);
  });

  /**
   * **A campaign the backend will re-transfer has to have a button that re-transfers it.**
   *
   * Every refusal after the phone is claimed — a route disagreement, a session that would not
   * open, an unmeasured build — ends the campaign in `failedBeforeDispatch`, and
   * `claim_publish_campaign_for_transfer` accepts exactly that state. The retry has to start at
   * Transfer rather than Post, because claiming an assignment overwrites its `evidence_json`
   * with the run intent and the `nativeImport.importId` that Post needs is gone.
   *
   * With Transfer shown only for `ready` and Post only for `imported`, the one state the
   * backend was built to let an operator retry had no button at all: the campaign was
   * retryable in the database and finished on the screen.
   */
  it("offers a full-pipeline retry for a campaign that failed before dispatch", async () => {
    const user = userEvent.setup();
    const list = vi.mocked(publishList);
    list.mockReset();
    executeCampaign.mockClear();
    list.mockResolvedValue([
      {
        id: "campaign-1",
        requestId: "req-1",
        sourceRoot: "C:/carousels",
        state: "failedBeforeDispatch",
        runAt: null,
        visibility: "public",
        cleanupPolicy: "afterPost",
        assignments: [],
        createdAt: "2026-08-18T00:00:00.000Z",
        updatedAt: "2026-08-18T00:00:00.000Z",
        errorCode: "post_refused_before_dispatch",
      },
    ] as never);

    render(<PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />);

    await user.click(screen.getByRole("tab", { name: "Theo dõi" }));
    const retry = await screen.findByRole("button", { name: "Chạy lại từ đầu" });
    await user.click(retry);
    expect(publishReconcile).toHaveBeenCalledWith("campaign-1");
    await waitFor(() => expect(executeCampaign).toHaveBeenCalled());
    expect(executeCampaign).toHaveBeenCalledWith("campaign-1", true);

    list.mockReset();
    list.mockResolvedValue([] as never);
  });

  it("stops a stale retry when reconciliation permits no further step", async () => {
    const user = userEvent.setup();
    const list = vi.mocked(publishList);
    list.mockReset();
    list.mockResolvedValue([{
      id: "campaign-locked",
      requestId: "req-locked",
      sourceRoot: "C:/carousels",
      state: "failedBeforeDispatch",
      runAt: null,
      visibility: "public",
      cleanupPolicy: "afterPost",
      assignments: [],
      createdAt: "2026-08-18T00:00:00.000Z",
      updatedAt: "2026-08-18T00:00:00.000Z",
      errorCode: "stale_projection",
    }] as never);
    vi.mocked(publishReconcile).mockResolvedValueOnce({
      campaignId: "campaign-locked",
      inputDigest: "approved-digest-1",
      status: "uncertain",
      retryScope: "none",
      reportJson: {},
      updatedAt: "2026-09-04T00:00:00Z",
    });

    render(<PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />);
    await user.click(screen.getByRole("tab", { name: "Theo dõi" }));
    await user.click(await screen.findByRole("button", { name: "Chạy lại từ đầu" }));

    expect(await screen.findByText(/không có bước nào được phép tự chạy lại/i)).toBeVisible();
    expect(executeCampaign).not.toHaveBeenCalled();
    expect(requestConfirm).not.toHaveBeenCalled();

    list.mockReset();
    list.mockResolvedValue([] as never);
  });

  /**
   * The preflight's own answer, on the page BEFORE the refusal. `readiness_of` runs adb
   * probes, so the fetch is keyed on the android udid set — an iOS-only roster (every
   * other test in this file) must not fetch at all, which is also what keeps those tests
   * free of act() noise.
   */
  it("shows each android phone's publish readiness, and only asks about android phones", async () => {
    const ready = vi.mocked(publishReadiness);
    ready.mockResolvedValueOnce([
      { udid: "ANDROID-A", readiness: { kind: "hierarchyReady" } },
      {
        udid: "ANDROID-B",
        readiness: { kind: "hierarchyMissing", labels: ["ComposerCaption", "PostButton"] },
      },
    ] as never);
    const android = (udid: string) =>
      ({ ...iphone(udid), platform: "android" }) as DeviceInfo;

    render(
      <PublishPage
        devices={[android("ANDROID-A"), android("ANDROID-B"), iphone("PHONE-A")]}
        selected={[]}
        onSelectUdids={() => {}}
      />,
    );

    // The positive chip says what was actually checked. `hierarchyReady` comes from the
    // shortest gap across the catalogue for that package — not from this phone's own
    // build — so a chip promising "sẵn sàng" outright would be a claim the backend never
    // made, and the phone would still be refused at the first tap after a TikTok update.
    await screen.findByText(/bản đo có đủ nhãn/);
    expect(screen.getByText(/chưa đối chiếu build máy/)).toBeTruthy();
    const missing = screen.getByText(/thiếu ô chú thích, nút Đăng/);
    expect(missing).toBeTruthy();
    expect(missing.closest(".pill")).not.toHaveAttribute("title");
    const technical = screen.getByText("Chi tiết khả năng tương thích").closest("details")!;
    expect(technical).not.toHaveAttribute("open");
    expect(within(technical).getByText("ComposerCaption, PostButton")).toBeInTheDocument();
    expect(ready).toHaveBeenCalledWith(["ANDROID-A", "ANDROID-B"]);
    expect(screen.getByText(/bị từ chối trước khi chuyển nội dung/)).toBeInTheDocument();
    expect(screen.queryByText(/composer_scout/)).toBeNull();
  });

  it("renders a future readiness variant as unknown and keeps its raw code in details", async () => {
    vi.mocked(publishReadiness).mockResolvedValueOnce([
      { udid: "ANDROID-A", readiness: { kind: "futureProbe", raw: 7 } },
    ] as never);
    const android = ({ ...iphone("ANDROID-A"), platform: "android" }) as DeviceInfo;

    render(<PublishPage devices={[android]} selected={[]} onSelectUdids={() => {}} />);

    const label = await screen.findByText(/trạng thái chưa nhận diện/);
    expect(label.closest(".pill")).not.toHaveAttribute("title");
    const technical = screen.getByText("Chi tiết khả năng tương thích").closest("details")!;
    expect(technical).not.toHaveAttribute("open");
    expect(within(technical).getByText(/futureProbe/)).toBeInTheDocument();
  });

  /**
   * A phone whose TikTok updates in place keeps its udid, so the effect's key cannot
   * notice the one change readiness is keyed on a build for. Without this button the only
   * way to re-ask was to unplug the phone.
   */
  it("re-asks readiness when the operator presses Hỏi lại", async () => {
    const ready = vi.mocked(publishReadiness);
    ready.mockReset();
    ready.mockResolvedValue([
      { udid: "ANDROID-A", readiness: { kind: "hierarchyReady" } },
    ] as never);
    const android = (udid: string) =>
      ({ ...iphone(udid), platform: "android" }) as DeviceInfo;
    const user = userEvent.setup();

    render(
      <PublishPage
        devices={[android("ANDROID-A")]}
        selected={[]}
        onSelectUdids={() => {}}
      />,
    );
    await screen.findByText(/bản đo có đủ nhãn/);
    const beforeClick = ready.mock.calls.length;

    await user.click(screen.getByRole("button", { name: "Hỏi lại" }));
    await waitFor(() => expect(ready.mock.calls.length).toBe(beforeClick + 1));

    ready.mockReset();
    ready.mockResolvedValue([] as never);
  });

  /**
   * `publishList` carries assignment PLANS, so per-phone state/errorCode were invisible
   * here — a campaign could sit `failedBeforeDispatch` with the one refusing phone
   * unnameable except by reading the backend log. The detail toggle is that read.
   */
  it("names the refusing phone when the operator opens a campaign's details", async () => {
    const user = userEvent.setup();
    const list = vi.mocked(publishList);
    list.mockResolvedValue([
      {
        id: "campaign-9",
        requestId: "req-9",
        sourceRoot: "C:/carousels",
        state: "failedBeforeDispatch",
        runAt: null,
        visibility: "public",
        cleanupPolicy: "afterPost",
        assignments: [],
        createdAt: "2026-08-18T00:00:00.000Z",
        updatedAt: "2026-08-18T00:00:00.000Z",
        errorCode: "post_refused_before_dispatch",
      },
    ] as never);
    vi.mocked(publishGet).mockResolvedValueOnce({
      campaign: { id: "campaign-9" },
      bundles: [],
      assignments: [
        {
          id: "asg-1",
          campaignId: "campaign-9",
          bundleId: "req-9:b1",
          ordinal: 0,
          udid: "PHONE-A",
          state: "failedBeforeDispatch",
          errorCode: "route_authorities_disagree",
          evidenceJson: JSON.stringify({
            post: { state: "posted" },
            cleanup: { state: "not_cleaned", message: "adb disconnected" },
          }),
        },
      ],
      events: [],
    } as never);
    vi.mocked(publishReconcile).mockResolvedValueOnce({
      campaignId: "campaign-9",
      inputDigest: "approved-digest-1",
      status: "partial",
      retryScope: "linkAndSheet",
      reportJson: {},
      updatedAt: "2026-09-04T00:00:00Z",
    });

    const namedDevices = [{ ...devices[0], name: "SM-G950F" }, ...devices.slice(1)];
    const metas = new Map<string, DeviceMeta>([
      [
        "PHONE-A",
        { udid: "PHONE-A", notes: "", tags: [], alias: "Máy quay sản phẩm", number: 17 },
      ],
    ]);
    render(
      <PublishPage
        devices={namedDevices}
        selected={[]}
        metas={metas}
        onSelectUdids={() => {}}
      />,
    );

    await user.click(screen.getByRole("tab", { name: "Theo dõi" }));
    await user.click(await screen.findByRole("button", { name: "Chi tiết máy" }));
    expect(publishReconcile).toHaveBeenCalledWith("campaign-9");
    expect(await screen.findByText("Chỉ tiếp tục lấy liên kết và ghi Sheet")).toBeVisible();
    const technical = await screen.findByRole("group", { name: "Chi tiết kỹ thuật máy" });
    const deviceCell = technical.closest("td")!;
    const row = deviceCell.closest("tr")!;
    expect(row).toHaveTextContent("Máy 17 · Máy quay sản phẩm");
    expect(row).toHaveTextContent("Dừng trước khi đăng");
    const raw = within(technical).getByText(/UDID: PHONE-A/);
    expect(raw).not.toBeVisible();
    await user.click(within(technical).getByText("Chi tiết"));
    expect(raw).toBeVisible();
    expect(raw).toHaveTextContent("failedBeforeDispatch");
    expect(raw).toHaveTextContent("route_authorities_disagree");
    expect(screen.getByText(/chưa dọn được ảnh tạm: adb disconnected/)).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Ẩn chi tiết máy" }));
    expect(screen.queryByText(/route_authorities_disagree/)).toBeNull();

    list.mockReset();
    list.mockResolvedValue([] as never);
  });

  /**
   * The unconfigured badge answers "why is my link still `pending`?" on the page where the
   * operator is looking — and it must come from a real answer, not render as a flash while
   * the config is still loading, and not linger once url + token are both set.
   */
  it("shows the Sheet-unconfigured badge only for a real unconfigured answer", async () => {
    const getConfig = vi.mocked(
      (await import("../api")).publishSheetGetConfig,
    );

    getConfig.mockResolvedValueOnce({ webhookUrl: "", hasToken: false });
    render(<PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />);
    expect(await screen.findByText("Sheet chờ cấu hình")).toBeVisible();
    cleanup();

    getConfig.mockResolvedValueOnce({
      webhookUrl: "https://script.google.com/macros/s/x/exec",
      hasToken: true,
    });
    render(<PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />);
    // **The anchor has to come from the second answer itself.** Waiting on
    // `getConfig` having been called proved nothing — the first render already called it,
    // so the assertion was satisfied by stale history while the page was still loading,
    // and during loading the badge is absent anyway. The page writes the config's URL into
    // the webhook field, so finding that value is proof this answer landed.
    await screen.findByDisplayValue("https://script.google.com/macros/s/x/exec");
    expect(screen.queryByText("Sheet chờ cấu hình")).toBeNull();
  });

  it("fails closed when Sheet config cannot be read and enables save only after retry", async () => {
    const getConfig = vi.mocked(publishSheetGetConfig);
    const saveConfig = vi.mocked(publishSheetSaveConfig);
    getConfig
      .mockReset()
      .mockRejectedValueOnce(new Error("credential store unavailable"))
      .mockResolvedValueOnce({
        webhookUrl: "https://script.google.com/macros/s/current/exec",
        hasToken: true,
      });

    render(<PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />);
    fireEvent.click(screen.getByText("Cấu hình Sheet"));

    expect(await screen.findByRole("alert")).toHaveTextContent("credential store unavailable");
    const save = screen.getByRole("button", { name: "Lưu cấu hình" });
    expect(save).toBeDisabled();
    fireEvent.click(save);
    expect(saveConfig).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Thử lại" }));
    await screen.findByDisplayValue("https://script.google.com/macros/s/current/exec");
    expect(save).not.toBeDisabled();
    expect(screen.queryByText("Không đọc được Sheet")).toBeNull();
  });
});
