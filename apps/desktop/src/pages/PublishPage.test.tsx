import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { listenRiviuEvents, publishList, publishTransfer } from "../api";
import { PublishPage } from "./PublishPage";
import { resetToasts } from "../toastStore";
import type { AppEvent, DeviceInfo, PublishBundle, PublishFolderManifest } from "../types";

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

vi.mock("../pickFile", () => ({
  pickDirectory: vi.fn(async () => "C:/carousels"),
  pickIpa: vi.fn(async () => null),
  pickMaterial: vi.fn(async () => null),
}));

vi.mock("../api", () => ({
  addAppLibrary: vi.fn(async () => undefined),
  addMaterial: vi.fn(async () => undefined),
  analyticsSummary: vi.fn(async () => ({})),
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
  publishList: vi.fn(async () => []),
  publishPrepare: vi.fn(async () => undefined),
  publishPost: vi.fn(async () => undefined),
  publishScanFolder: vi.fn(async () => manifest),
  publishTransfer: vi.fn(async () => undefined),
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
  resetToasts();
});

afterEach(cleanup);

describe("publish, bundle to phone", () => {
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

    await user.click(screen.getByRole("button", { name: /Tạo & chuẩn bị/ }));
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
    await waitFor(() => expect(screen.getByText("succeeded")).toBeTruthy());
    releasePosting();

    // The late answer is discarded rather than rendered. Waiting first would pass even
    // without the guard, so this settles the microtask queue and then looks.
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(
      screen.queryByText("posting"),
      "a reload that started earlier repainted the page over a newer one",
    ).toBeNull();
    expect(screen.getByText("succeeded")).toBeTruthy();

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
  it("offers a re-transfer for a campaign that failed before dispatch", async () => {
    const user = userEvent.setup();
    const list = vi.mocked(publishList);
    const transfer = vi.mocked(publishTransfer);
    list.mockReset();
    transfer.mockClear();
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

    const retry = await screen.findByRole("button", { name: "Chuyển lại media" });
    await user.click(retry);
    await waitFor(() => expect(transfer).toHaveBeenCalledWith("campaign-1"));

    list.mockReset();
    list.mockResolvedValue([] as never);
  });
});
