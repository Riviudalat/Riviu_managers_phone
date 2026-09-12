import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  listenRiviuEvents,
  operationListRuns,
  publishGet,
  publishList,
  publishCancel,
  publishReconcile,
  publishScanFolder,
  publishSheetGetConfig,
  publishSheetPrepare,
} from "../api";
import { PublishPage } from "./PublishPage";
import { requestConfirm } from "../confirmStore";
import { requestWorkspaceLeave } from "../workspaceDraft";
import { resetToasts } from "../toastStore";
import { pickDirectory } from "../pickFile";
import { writeFormDraft } from "../formDraftStorage";
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
  detail: {
    campaign: { id: "campaign-1" },
    bundles: [],
    assignments: [],
    events: [],
  },
}));

const preflightCampaign = vi.fn(
  async (
    request: PublishPreflightRequest,
  ): Promise<PublishPreflightReport> => ({
    inputDigest: "approved-digest-1",
    targetSnapshot: {
      targetRef: request.targetRef ?? {
        type: "explicit",
        udids: request.udids,
      },
      included: request.udids.map((udid, index) => ({
        udid,
        alias: `Máy ${index + 1}`,
        number: index + 1,
      })),
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
  }),
);

vi.mock("../pickFile", () => ({
  pickDirectory: vi.fn(async () => "C:/carousels"),
  pickIpa: vi.fn(async () => null),
  pickMaterial: vi.fn(async () => null),
}));

vi.mock("../confirmStore", () => ({
  useConfirmRequest: () => null,
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
  automationGet: vi.fn(),
  automationScheduleList: vi.fn(async () => []),
  operationListRuns: vi.fn(async () => []),
  operationGetRun: vi.fn(async () => null),
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
  publishImagePreview: vi.fn(async () => "data:image/jpeg;base64,AA=="),
  publishAutoAssign: vi.fn(async () => ({ plan: [] })),
  listAppsLibrary: vi.fn(async () => []),
  listGroups: vi.fn(async () => []),
  listMaterials: vi.fn(async () => []),
  listSchedules: vi.fn(async () => []),
  listScripts: vi.fn(async () => []),
  publishCancel: vi.fn(async () => undefined),
  publishCreateCampaign: (...args: unknown[]) =>
    createCampaign(...(args as [])),
  publishExecute: (...args: unknown[]) => executeCampaign(...(args as [])),
  publishGet: vi.fn(async () => null),
  publishList: vi.fn(async () => []),
  publishPreflight: (...args: unknown[]) =>
    preflightCampaign(...(args as [PublishPreflightRequest])),
  publishReadiness: vi.fn(async () => []),
  publishReconcile: vi.fn(async (campaignId: string) => ({
    campaignId,
    inputDigest: "approved-digest-1",
    status: "partial",
    retryScope: "fullPipeline",
    reportJson: {},
    updatedAt: "2026-09-04T00:00:00Z",
  })),
  publishSheetGetConfig: vi.fn(async () => ({
    webhookUrl: "",
    hasToken: false,
  })),
  publishSheetPrepare: vi.fn(),
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
  vi.mocked(publishList).mockReset().mockResolvedValue([]);
  HTMLDialogElement.prototype.showModal = function () {
    this.setAttribute("open", "");
  };
  HTMLDialogElement.prototype.close = function () {
    this.removeAttribute("open");
  };
  vi.mocked(operationListRuns).mockReset().mockResolvedValue([]);
  createCampaign.mockClear();
  executeCampaign.mockClear();
  preflightCampaign.mockClear();
  vi.mocked(publishReconcile)
    .mockReset()
    .mockImplementation(async (campaignId) => ({
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
  vi.mocked(publishSheetPrepare).mockReset();
  vi.mocked(publishScanFolder).mockReset().mockResolvedValue(manifest);
  vi.mocked(pickDirectory).mockReset().mockResolvedValue("C:/carousels");
  resetToasts();
});

afterEach(cleanup);

async function prepareOne() {
  await userEvent.click(screen.getByRole("button", { name: "Chọn thư mục" }));
  await userEvent.click(screen.getByRole("button", { name: "Quét" }));
  await userEvent.click(
    await screen.findByRole("checkbox", { name: "Chọn bo1" }),
  );
  await userEvent.click(screen.getByRole("button", { name: "Chọn nhanh" }));
}

describe("production publish wizard", () => {
  it("selects the newly created campaign after entering Setup from an older operation", async () => {
    const oldCampaign = { id: "old-operation", requestId: "old", sourceRoot: "C:/old-source", state: "succeeded", assignments: [], createdAt: "2026-09-09T00:00:00Z", updatedAt: "2026-09-09T00:00:00Z" };
    const newCampaign = { ...oldCampaign, id: "campaign-1", sourceRoot: "C:/carousels", state: "prepared" };
    vi.mocked(publishGet).mockResolvedValueOnce({ campaign: oldCampaign, bundles: [], events: [], assignments: [] } as never);
    vi.mocked(publishList).mockResolvedValue([oldCampaign] as never);
    createCampaign.mockImplementationOnce(async () => {
      vi.mocked(publishList).mockResolvedValue([oldCampaign, newCampaign] as never);
      return newCampaign as never;
    });
    render(<PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} operationSource={{ operationId: "publish:old-operation", sourceId: "old-operation", kind: "publish" }} />);
    await screen.findByRole("button", { name: "Ẩn chi tiết máy" });
    await userEvent.click(screen.getByRole("tab", { name: "Thiết lập" }));
    await prepareOne();
    await userEvent.click(screen.getByRole("button", { name: "Kiểm tra & đăng" }));
    const confirm = await screen.findByRole("button", { name: "Xác nhận đăng 1 bài" });
    await waitFor(() => expect(confirm).toBeEnabled());
    await userEvent.click(confirm);
    await waitFor(() => expect(document.querySelector(".publish-monitor-detail-head")).toHaveTextContent("Chiến dịch 2"));
    expect(document.querySelector(".publish-monitor-detail-head")).toHaveTextContent("carousels");
    expect(executeCampaign).toHaveBeenCalledWith("campaign-1", true);
  });
  it("ignores a legacy hidden future runAt when publishing immediately from Setup", async () => {
    writeFormDraft("publish", { sourceRoot: "C:/carousels", bundleIds: ["b1"],
      assignments: { b1: "PHONE-A" }, captionDrafts: { b1: "caption for bo1" },
      runAt: "2099-09-09T12:00", soundPolicyOverride: null, sheetEnabled: false, deleteAfterPublish: false });
    render(<PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />);
    await waitFor(() => expect(screen.getByRole("checkbox", { name: "Chọn bo1" })).toBeChecked());
    await userEvent.click(screen.getByRole("button", { name: "Kiểm tra & đăng" }));
    const confirm = await screen.findByRole("button", { name: "Xác nhận đăng 1 bài" });
    await waitFor(() => expect(confirm).toBeEnabled());
    expect(preflightCampaign).toHaveBeenLastCalledWith(expect.objectContaining({ runAt: null }));
    await userEvent.click(confirm);
    await waitFor(() => expect(createCampaign).toHaveBeenCalledOnce());
    expect((createCampaign.mock.calls[0] as unknown[])[3]).toBeNull();
    expect(executeCampaign).toHaveBeenCalledWith("campaign-1", true);
  });
  it("autosaves mapping and caption, rescans on remount and requires fresh preflight", async () => {
    const view = render(<PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />);
    await prepareOne();
    await userEvent.click(screen.getByRole("checkbox", { name: "Xóa bản chuyển sau khi đăng thành công" }));
    await act(async () => { expect(await requestWorkspaceLeave()).toBe(true); });
    expect(requestConfirm).not.toHaveBeenCalled();
    expect(createCampaign).not.toHaveBeenCalled();
    view.unmount();
    render(<PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />);
    await waitFor(() => expect(screen.getByRole("checkbox", { name: "Chọn bo1" })).toBeChecked());
    expect(publishScanFolder).toHaveBeenCalledTimes(2);
    expect(screen.getByRole("combobox", { name: "Máy nhận bài đang chỉnh" })).toHaveValue("PHONE-A");
    expect(screen.getByRole("checkbox", { name: "Xóa bản chuyển sau khi đăng thành công" })).toBeChecked();
    expect(preflightCampaign).not.toHaveBeenCalled();
    expect(executeCampaign).not.toHaveBeenCalled();
  });
  it("scans real source but never selects or dispatches on load", async () => {
    render(
      <PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Chọn thư mục" }));
    await userEvent.click(screen.getByRole("button", { name: "Quét" }));
    expect(
      await screen.findByRole("checkbox", { name: "Chọn bo1" }),
    ).not.toBeChecked();
    expect(screen.getByRole("button", { name: "Kiểm tra & đăng" })).toBeDisabled();
    expect(createCampaign).not.toHaveBeenCalled();
    expect(preflightCampaign).not.toHaveBeenCalled();
  });
  it("keeps scan errors and ignores late success after source edit", async () => {
    let release!: (value: PublishFolderManifest) => void;
    vi.mocked(publishScanFolder).mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          release = resolve;
        }),
    );
    render(
      <PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Chọn thư mục" }));
    await userEvent.click(screen.getByRole("button", { name: "Quét" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Thư mục nguồn" }), {
      target: { value: "C:/new-source" },
    });
    await act(async () => {
      release(manifest);
    });
    expect(screen.queryByRole("checkbox", { name: "Chọn bo1" })).toBeNull();
    expect(screen.getByRole("textbox", { name: "Thư mục nguồn" })).toHaveValue(
      "C:/new-source",
    );
    vi.mocked(publishScanFolder).mockRejectedValueOnce(
      new Error("publish folder has no bundle directories"),
    );
    await userEvent.click(screen.getByRole("button", { name: "Quét" }));
    expect(
      await screen.findByText("Chưa tìm thấy gói bài trong thư mục đã chọn"),
    ).toBeVisible();
  });
  it("posts only after current preflight and explicit confirmation, carrying Sheet and cleanup", async () => {
    render(
      <PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />,
    );
    await prepareOne();
    await userEvent.click(
      screen.getByRole("checkbox", { name: "Xóa bản chuyển sau khi đăng thành công" }),
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Kiểm tra & đăng" }),
    );
    const confirm = await screen.findByRole("button", {
      name: "Xác nhận đăng 1 bài",
    });
    await waitFor(() => expect(confirm).toBeEnabled());
    expect(createCampaign).not.toHaveBeenCalled();
    expect(preflightCampaign).toHaveBeenLastCalledWith(
      expect.objectContaining({
        bundleIds: ["b1"],
        udids: ["PHONE-A"],
        deleteAfterPublish: true,
        sheetEnabled: false,
      }),
    );
    await userEvent.click(confirm);
    await waitFor(() => expect(createCampaign).toHaveBeenCalledTimes(1));
    expect(createCampaign).toHaveBeenCalledWith(
      "C:/carousels",
      ["b1"],
      ["PHONE-A"],
      null,
      { b1: "caption for bo1" },
      expect.any(Object),
      { type: "explicit", udids: ["PHONE-A"] },
      true,
      "approved-digest-1",
      false,
      true,
    );
    expect(executeCampaign).toHaveBeenCalledWith("campaign-1", true);
    expect(requestConfirm).toHaveBeenCalledWith(
      expect.objectContaining({
        message: expect.stringContaining("sau khi mở TikTok"),
      }),
    );
  });
  it("keeps unsupported remote TikTok preflight visible and never creates or posts", async () => {
    const implementation = preflightCampaign.getMockImplementation()!;
    preflightCampaign.mockImplementationOnce(async (request) => {
      const report = await implementation(request);
      const issue = {
        code: "composer_unmeasured",
        udid: "PHONE-A",
        bundleId: "b1",
        message: "composer chưa đủ locator cho đúng package/build/locale này",
      };
      return {
        ...report,
        canExecute: false,
        issues: [issue],
        assignments: report.assignments.map((row) => ({
          ...row,
          packageName: "com.zhiliaoapp.musically",
          version: "45.7.3",
          locale: "en-US",
          composer: "fail" as const,
          issues: [issue],
        })),
      };
    });
    render(<PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />);
    await prepareOne();
    await userEvent.click(screen.getByRole("button", { name: "Kiểm tra & đăng" }));
    expect(await screen.findByText(/TikTok quốc tế · Phiên bản 45.7.3/)).toBeVisible();
    expect(screen.getByText("Chưa hỗ trợ luồng đăng trên bản TikTok này")).toBeVisible();
    const confirm = screen.getByRole("button", { name: "Xác nhận đăng 1 bài" });
    expect(confirm).toBeDisabled();
    await userEvent.click(confirm);
    expect(createCampaign).not.toHaveBeenCalled();
    expect(executeCampaign).not.toHaveBeenCalled();
  });
  it("rejects a late preflight when its assigned machine leaves the scope", async () => {
    let release!: (value: PublishPreflightReport) => void;
    const implementation = preflightCampaign.getMockImplementation()!;
    preflightCampaign.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          release = resolve;
        }),
    );
    const view = render(
      <PublishPage
        devices={devices}
        selected={[]}
        targetUdids={["PHONE-A"]}
        onSelectUdids={() => {}}
      />,
    );
    await prepareOne();
    await userEvent.click(
      screen.getByRole("button", { name: "Kiểm tra & đăng" }),
    );
    const request = preflightCampaign.mock.calls.at(-1)![0];
    await userEvent.click(screen.getByRole("button", { name: "Đóng" }));
    view.rerender(
      <PublishPage
        devices={devices}
        selected={[]}
        targetUdids={["PHONE-B"]}
        onSelectUdids={() => {}}
      />,
    );
    await act(async () => {
      release(await implementation(request));
    });
    expect(
      screen.queryByText("Đầu vào đã đạt kiểm tra. Chưa đăng bài."),
    ).toBeNull();
    expect(
      screen.getByRole("button", { name: "Kiểm tra & đăng" }),
    ).toBeDisabled();
    expect(createCampaign).not.toHaveBeenCalled();
  });
  it("does not create a campaign if the roster changes during native confirmation", async () => {
    let approve!: (value: boolean) => void;
    vi.mocked(requestConfirm).mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          approve = resolve;
        }),
    );
    const view = render(
      <PublishPage
        devices={devices}
        selected={[]}
        targetUdids={["PHONE-A"]}
        onSelectUdids={() => {}}
      />,
    );
    await prepareOne();
    await userEvent.click(
      screen.getByRole("button", { name: "Kiểm tra & đăng" }),
    );
    const confirm = await screen.findByRole("button", {
      name: "Xác nhận đăng 1 bài",
    });
    await waitFor(() => expect(confirm).toBeEnabled());
    await userEvent.click(confirm);
    view.rerender(
      <PublishPage
        devices={devices}
        selected={[]}
        targetUdids={["PHONE-B"]}
        onSelectUdids={() => {}}
      />,
    );
    await act(async () => approve(true));
    expect(createCampaign).not.toHaveBeenCalled();
    expect(executeCampaign).not.toHaveBeenCalled();
  });
  it("shows one folder picker beside scan without profiles or settings tab", async () => {
    render(<PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />);
    expect(screen.queryByText("Hồ sơ & cài đặt", { exact: true })).toBeNull();
    expect(screen.queryByRole("button", { name: "Nhập nội dung" })).toBeNull();
    expect(screen.queryByRole("tab", { name: "Cài đặt" })).toBeNull();
    expect(screen.getAllByRole("button", { name: "Chọn thư mục" })).toHaveLength(1);
    await userEvent.click(screen.getByRole("button", { name: "Chọn thư mục" }));
    expect(screen.getByRole("textbox", { name: "Thư mục nguồn" })).toHaveValue("C:/carousels");
    expect(publishScanFolder).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "Quét" }));
    await waitFor(() => expect(publishScanFolder).toHaveBeenCalledWith("C:/carousels"));
    expect(createCampaign).not.toHaveBeenCalled();
  });
  it("does not publish when native confirmation is cancelled", async () => {
    vi.mocked(requestConfirm).mockResolvedValueOnce(false);
    render(
      <PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />,
    );
    await prepareOne();
    await userEvent.click(
      screen.getByRole("button", { name: "Kiểm tra & đăng" }),
    );
    const confirm = await screen.findByRole("button", {
      name: "Xác nhận đăng 1 bài",
    });
    await waitFor(() => expect(confirm).toBeEnabled());
    await userEvent.click(confirm);
    expect(createCampaign).not.toHaveBeenCalled();
  });
  it("changing cleanup invalidates the approved digest", async () => {
    render(
      <PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />,
    );
    await prepareOne();
    await userEvent.click(
      screen.getByRole("button", { name: "Kiểm tra & đăng" }),
    );
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Xác nhận đăng 1 bài" }),
      ).toBeEnabled(),
    );
    await userEvent.click(screen.getByRole("button", { name: "Đóng" }));
    await userEvent.click(
      screen.getByRole("checkbox", { name: "Xóa bản chuyển sau khi đăng thành công" }),
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Kiểm tra & đăng" }),
    );
    await waitFor(() => expect(preflightCampaign).toHaveBeenCalledTimes(2));
    expect(preflightCampaign.mock.calls.at(-1)![0].deleteAfterPublish).toBe(
      true,
    );
  });
  it("keeps bundle identity when a sibling is removed", async () => {
    render(
      <PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Chọn thư mục" }));
    await userEvent.click(screen.getByRole("button", { name: "Quét" }));
    await userEvent.click(
      await screen.findByRole("checkbox", { name: "Chọn bo1" }),
    );
    await userEvent.click(screen.getByRole("checkbox", { name: "Chọn bo2" }));
      await userEvent.click(screen.getByRole("button", { name: "Chọn nhanh" }));
    await userEvent.click(screen.getByRole("checkbox", { name: "Chọn bo1" }));
      await userEvent.click(
      screen.getByRole("button", { name: "Kiểm tra & đăng" }),
    );
    expect(preflightCampaign).toHaveBeenLastCalledWith(
      expect.objectContaining({ bundleIds: ["b2"], udids: ["PHONE-B"] }),
    );
  });
  it("preserves edited caption without modifying source manifest", async () => {
    render(
      <PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Chọn thư mục" }));
    await userEvent.click(screen.getByRole("button", { name: "Quét" }));
    await screen.findByRole("textbox", { name: "Nội dung bài đăng" });
    fireEvent.change(
      screen.getByRole("textbox", { name: "Nội dung bài đăng" }),
      { target: { value: "Nội dung mới" } },
    );
    expect(manifest.bundles[0].caption).toBe("caption for bo1");
    await userEvent.click(screen.getByRole("checkbox", { name: "Chọn bo1" }));
      await userEvent.click(screen.getByRole("button", { name: "Chọn nhanh" }));
    await userEvent.click(
      screen.getByRole("button", { name: "Kiểm tra & đăng" }),
    );
    expect(preflightCampaign).toHaveBeenLastCalledWith(
      expect.objectContaining({ captionOverrides: { b1: "Nội dung mới" } }),
    );
  });
});

describe("publish campaign monitoring", () => {
  it("can cancel transfer before Post through the existing cancellation command", async () => {
    const campaign = { id: "transferring", requestId: "r", sourceRoot: "C:/fixture", state: "transferring", assignments: [], createdAt: "2026-09-10T00:00:00Z" };
    vi.mocked(publishList).mockResolvedValue([campaign] as never);
    vi.mocked(publishGet).mockResolvedValue({campaign,bundles:[],assignments:[],events:[]} as never);
    render(<PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />);
    fireEvent.click(screen.getByRole("tab", {name:"Theo dõi"}));
    fireEvent.click(await screen.findByRole("button", {name:"Chi tiết máy"}));
    fireEvent.click(await screen.findByRole("button", {name:"Huỷ"}));
    await waitFor(() => expect(publishCancel).toHaveBeenCalledWith("transferring"));
    expect(executeCampaign).not.toHaveBeenCalled();
  });

  it("opens an exact historical campaign without reconciling or executing it", async () => {
    const campaign = {
      id: "historical",
      requestId: "req",
      sourceRoot: "C:/fixture",
      state: "failed",
      assignments: [],
      createdAt: "2026-08-01T00:00:00Z",
    };
    vi.mocked(publishGet).mockResolvedValueOnce({
      campaign,
      bundles: [],
      assignments: [],
      events: [],
    } as never);
    render(
      <PublishPage
        devices={devices}
        selected={[]}
        onSelectUdids={() => {}}
        operationSource={{
          operationId: "publish:historical",
          sourceId: "historical",
          kind: "publish",
        }}
      />,
    );
    await waitFor(() => expect(publishGet).toHaveBeenCalledWith("historical"));
    expect(screen.getByRole("tabpanel", { name: "Theo dõi" })).toBeVisible();
    expect(
      await screen.findByRole("region", {
        name: "Chi tiết chiến dịch đang chọn",
      }),
    ).toBeVisible();
    expect(publishReconcile).not.toHaveBeenCalled();
    expect(executeCampaign).not.toHaveBeenCalled();
  });

  it("reports a missing exact campaign without showing another campaign instead", async () => {
    vi.mocked(publishGet).mockResolvedValueOnce(null);
    render(
      <PublishPage
        devices={devices}
        selected={[]}
        onSelectUdids={() => {}}
        operationSource={{
          operationId: "publish:missing",
          sourceId: "missing",
          kind: "publish",
        }}
      />,
    );
    expect(
      await screen.findByText(
        "Chiến dịch được chọn không còn trong nguồn dữ liệu.",
      ),
    ).toBeVisible();
    expect(
      screen.queryByRole("table", { name: "Danh sách chiến dịch" }),
    ).toBeNull();
    expect(executeCampaign).not.toHaveBeenCalled();
  });

  it("keeps a confirmed Post partial and offers only the outstanding Sheet step", async () => {
    const campaign = {
      id: "posted-partial",
      requestId: "request",
      sourceRoot: "C:/fixture",
      state: "succeeded",
      visibility: "public",
      cleanupPolicy: "deleteImportedAssetsAfterVerified",
      assignments: [],
      createdAt: "2026-09-05T00:00:00Z",
      updatedAt: "2026-09-05T00:00:00Z",
    };
    vi.mocked(publishList).mockResolvedValueOnce([campaign] as never);
    vi.mocked(operationListRuns).mockResolvedValueOnce([
      {
        id: "publish:posted-partial",
        sourceId: "posted-partial",
        kind: "publish",
        title: "Đăng bài",
        state: "partial",
        targetCount: 1,
        totalItems: 1,
        completedItems: 1,
        issueCount: 1,
        retryableCount: 1,
        retryScope: "sheetOnly",
        createdAt: campaign.createdAt,
        updatedAt: campaign.updatedAt,
      },
    ]);
    vi.mocked(publishReconcile).mockResolvedValue({
      campaignId: campaign.id,
      inputDigest: "digest",
      status: "partial",
      retryScope: "sheetOnly",
      reportJson: {},
      updatedAt: campaign.updatedAt,
    });
    render(
      <PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />,
    );
    fireEvent.click(screen.getByRole("tab", { name: "Theo dõi" }));
    expect(await screen.findByText("Hoàn tất một phần")).toBeVisible();
    expect(within(screen.getByRole("list", { name: "Chiến dịch đăng bài" })).queryByText("Hoàn tất", { exact: true })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Chi tiết máy" }));
    fireEvent.click(screen.getByRole("button", { name: "Ghi lại Sheet" }));
    await waitFor(() =>
      expect(executeCampaign).toHaveBeenCalledWith(campaign.id, true),
    );
    expect(requestConfirm).toHaveBeenCalledWith(
      expect.objectContaining({
        message: expect.stringContaining("Chỉ tiếp tục ghi Sheet"),
      }),
    );
  });

  it("checks the inline Sheet link without dispatching or claiming write access", async () => {
    const url = "https://docs.google.com/spreadsheets/d/fixture/edit#gid=0";
    vi.mocked(publishSheetPrepare).mockResolvedValue({ sheetUrl: url, spreadsheetId: "fixture", sheetGid: 0,
      readable: true, connectionVerified: false, layout: "compact", columns: [], message: "Đọc được bảng; chưa xác minh kết nối ghi." });
    render(<PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />);
    fireEvent.change(screen.getByRole("textbox", { name: "Link Google Sheet" }), { target: { value: url } });
    fireEvent.click(screen.getByRole("button", { name: "Kết nối Sheet" }));
    await waitFor(() => expect(publishSheetPrepare).toHaveBeenCalledWith(url));
    expect(await screen.findByText("Đọc được bảng; chưa xác minh kết nối ghi.")).toBeVisible();
    expect(createCampaign).not.toHaveBeenCalled();
    expect(executeCampaign).not.toHaveBeenCalled();
  });

  it("keeps the checked Sheet URL fixed until its response then invalidates it on edit", async () => {
    let answer!: (value: never) => void;
    vi.mocked(publishSheetPrepare).mockImplementation(() => new Promise(resolve => { answer = resolve; }));
    render(<PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />);
    const input = screen.getByRole("textbox", { name: "Link Google Sheet" });
    fireEvent.change(input, { target: { value: "https://docs.google.com/spreadsheets/d/old/edit" } });
    fireEvent.click(screen.getByRole("button", { name: "Kết nối Sheet" }));
    expect(input).toBeDisabled();
    await userEvent.type(input, "changed");
    expect(input).toHaveValue("https://docs.google.com/spreadsheets/d/old/edit");
    await act(async () => answer({ readable: true, connectionVerified: true, message: "Old verified result" } as never));
    expect(await screen.findByText("Old verified result")).toBeVisible();
    expect(input).toBeEnabled();
    fireEvent.change(input, { target: { value: "https://docs.google.com/spreadsheets/d/new/edit" } });
    expect(screen.queryByText("Old verified result")).toBeNull();
  });

  it("requires a verified Sheet connection and invalidates publish preflight after editing its URL", async () => {
    const url = "https://docs.google.com/spreadsheets/d/current/edit#gid=0";
    vi.mocked(publishSheetPrepare).mockResolvedValueOnce({ sheetUrl: url, spreadsheetId: "current", sheetGid: 0,
      readable: true, connectionVerified: false, layout: "internal", columns: [], message: "Đọc được bảng, chưa xác minh ghi" });
    vi.mocked(publishSheetPrepare).mockResolvedValueOnce({ sheetUrl: url, spreadsheetId: "current", sheetGid: 0,
      readable: true, connectionVerified: true, layout: "internal", columns: [], message: "Đã xác minh kết nối" });
    render(<PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />);
    await prepareOne();
    await userEvent.click(screen.getByRole("checkbox", { name: "Ghi kết quả lên Sheet" }));
    const input = screen.getByRole("textbox", { name: "Link Google Sheet" });
    fireEvent.change(input, { target: { value: url } });
    const publish = screen.getByRole("button", { name: "Kiểm tra & đăng" });
    expect(publish).toBeDisabled();
    await userEvent.click(screen.getByRole("button", { name: "Kết nối Sheet" }));
    await screen.findByText("Đọc được bảng, chưa xác minh ghi");
    expect(publish).toBeDisabled();
    expect(preflightCampaign).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "Kết nối Sheet" }));
    await waitFor(() => expect(publish).toBeEnabled());
    await userEvent.click(publish);
    await waitFor(() => expect(screen.getByRole("button", { name: "Xác nhận đăng 1 bài" })).toBeEnabled());
    await userEvent.click(screen.getByRole("button", { name: "Đóng" }));
    fireEvent.change(input, { target: { value: "https://docs.google.com/spreadsheets/d/other/edit#gid=0" } });
    expect(publish).toBeDisabled();
    expect(screen.queryByRole("button", { name: "Xác nhận đăng 1 bài" })).toBeNull();
    expect(createCampaign).not.toHaveBeenCalled();
    expect(executeCampaign).not.toHaveBeenCalled();
  });

  it("shows independent upload and post phases together", async () => {
    const campaign = { id: "parallel", requestId: "r", sourceRoot: "C:/fixture", state: "posting", visibility: "public", cleanupPolicy: "keepImportedAssets", assignments: [], createdAt: "2026-09-10T00:00:00Z", updatedAt: "2026-09-10T00:00:00Z" };
    vi.mocked(publishList).mockResolvedValue([campaign] as never);
    vi.mocked(publishGet).mockResolvedValue({campaign,bundles:[],events:[],assignments:["transferring","imported","posting","verifying"].map((state,i)=>({id:String(i),campaignId:campaign.id,bundleId:String(i),ordinal:i,udid:"PHONE-"+i,state}))} as never);
    render(<PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />);
    fireEvent.click(screen.getByRole("tab", {name:"Theo dõi"}));
    fireEvent.click(await screen.findByRole("button", {name:"Chi tiết máy"}));
    const counts=await screen.findByRole("status",{name:"Tiến độ từng máy"});
    for(const label of ["1 máy đang tải","1 máy đã tải, chờ đăng","1 máy đang gửi bài","1 máy chờ liên kết"]) expect(within(counts).getByText(label)).toBeVisible();
    expect(executeCampaign).not.toHaveBeenCalled();
  });

  it("keeps submitted posts pending even when an older execution snapshot says complete", async () => {
    const campaign = {
      id: "submitted-pending", requestId: "request", sourceRoot: "C:/fixture",
      state: "verifying", visibility: "public", cleanupPolicy: "keepImportedAssets",
      assignments: [{ bundleId: "bundle", udid: "PHONE-A", ordinal: 0 }],
      createdAt: "2026-09-09T00:00:00Z", updatedAt: "2026-09-09T00:01:00Z",
      errorCode: "post_verification_pending",
    };
    vi.mocked(publishList).mockResolvedValue([campaign] as never);
    vi.mocked(publishGet).mockResolvedValueOnce({
      campaign, bundles: [], events: [], assignments: [{
        id: "assignment", campaignId: campaign.id, bundleId: "bundle", ordinal: 0,
        udid: "PHONE-A", state: "verifying", errorCode: "post_verification_pending",
        evidenceJson: JSON.stringify({ post: { state: "submitted" }, cleanup: { state: "kept", appCleanup: { state: "leftRunning" } } }),
      }],
    } as never);
    vi.mocked(publishReconcile).mockResolvedValueOnce({
      campaignId: campaign.id, inputDigest: "digest", status: "complete", retryScope: "none",
      reportJson: { sheetEnabled: true }, updatedAt: "2026-09-09T00:00:00Z",
    });
    render(<PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />);
    fireEvent.click(screen.getByRole("tab", { name: "Theo dõi" }));
    expect(await screen.findByText("Đã bấm Đăng · chờ xác minh")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Chi tiết máy" }));
    const panel = await screen.findByRole("region", { name: "Chi tiết chiến dịch đang chọn" });
    expect(within(panel).getByText("Đang chờ xác minh bài đăng")).toBeVisible();
    expect(within(panel).getByText("Sheet chờ liên kết đã xác minh")).toBeVisible();
    expect(within(panel).getByText("đã giữ nội dung và để TikTok tiếp tục xử lý")).toBeVisible();
    expect(screen.queryByText("Đã hoàn tất", { exact: true })).toBeNull();
    expect(screen.queryByText("Sheet đã xác nhận", { exact: true })).toBeNull();
    expect(screen.queryByRole("link", { name: "Mở bài đã xác nhận" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Chạy lại từ đầu" })).toBeNull();
    expect(executeCampaign).not.toHaveBeenCalled();
  });

  it("shows periodic scheduled link checks without treating the waiting post as failure", async () => {
    const campaign = {
      id: "submitted-pending", requestId: "request", sourceRoot: "C:/fixture", runAt: "2026-09-09T07:00:00",
      state: "verifying", visibility: "public", cleanupPolicy: "keepImportedAssets",
      assignments: [{ bundleId: "bundle", udid: "PHONE-A", ordinal: 0 }],
      createdAt: "2026-09-09T00:00:00Z", updatedAt: "2026-09-09T00:01:00Z",
      errorCode: "post_verification_pending",
    };
    vi.mocked(publishList).mockResolvedValue([campaign] as never);
    vi.mocked(publishGet).mockResolvedValueOnce({
      campaign, bundles: [], events: [], assignments: [{
        id: "assignment", campaignId: campaign.id, bundleId: "bundle", ordinal: 0,
        udid: "PHONE-A", state: "verifying", errorCode: "post_verification_pending",
        evidenceJson: JSON.stringify({ post: { state: "submitted" }, verificationStatus: { state: "pending", reason: "TikTok đang xử lý; kiểm tra lại mỗi 5 phút, tối đa 4 giờ", checkedAt: "2026-09-09T00:40:00Z", nextCheckAt: "2026-09-09T00:45:00Z", reviewAfterMinutes: 240 }, cleanup: { state: "kept", appCleanup: { state: "leftRunning" } } }),
      }],
    } as never);
    vi.mocked(publishReconcile).mockResolvedValueOnce({
      campaignId: campaign.id, inputDigest: "digest", status: "complete", retryScope: "none",
      reportJson: { sheetEnabled: true }, updatedAt: "2026-09-09T00:00:00Z",
    });
    render(<PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />);
    fireEvent.click(screen.getByRole("tab", { name: "Theo dõi" }));
    expect(await screen.findByText("Đã bấm Đăng · chờ xác minh")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Chi tiết máy" }));
    const panel = await screen.findByRole("region", { name: "Chi tiết chiến dịch đang chọn" });
    expect(within(panel).getByText("Đang chờ xác minh bài đăng")).toBeVisible();
    expect(within(panel).getByText("Sheet chờ liên kết đã xác minh")).toBeVisible();
    expect(within(panel).getByText("đã giữ nội dung và để TikTok tiếp tục xử lý")).toBeVisible();
    expect(screen.queryByText("Đã hoàn tất", { exact: true })).toBeNull();
    expect(screen.queryByText("Sheet đã xác nhận", { exact: true })).toBeNull();
    expect(screen.queryByRole("link", { name: "Mở bài đã xác nhận" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Chạy lại từ đầu" })).toBeNull();
    expect(within(panel).getByText(/TikTok đang xử lý; kiểm tra lại mỗi 5 phút, tối đa 4 giờ/)).toBeVisible();
    expect(within(panel).getByText(/Kiểm tra tiếp:/)).toBeVisible();
    expect(within(panel).getByText(/Ngân sách tự kiểm: 240 phút/)).toBeVisible();
    expect(executeCampaign).not.toHaveBeenCalled();
  });

  it("shows expired publication as needs review and permits only an explicit link check", async () => {
    const campaign = {
      id: "review-post", requestId: "request", sourceRoot: "C:/fixture", state: "uncertain",
      errorCode: "post_verification_needs_review", visibility: "public", cleanupPolicy: "keepImportedAssets",
      assignments: [{ bundleId: "bundle", udid: "PHONE-A", ordinal: 0 }],
      createdAt: "2026-09-09T00:00:00Z", updatedAt: "2026-09-09T00:31:00Z",
    };
    const detail = { campaign, bundles: [], events: [], assignments: [{
      id: "assignment", campaignId: campaign.id, bundleId: "bundle", udid: "PHONE-A", ordinal: 0,
      state: "uncertain", errorCode: "post_verification_needs_review",
      evidenceJson: JSON.stringify({ verificationStatus: { state: "needsReview", reason: "Hồ sơ có bản nháp; chưa tìm thấy bài đã gửi." } }),
    }] };
    vi.mocked(publishList).mockResolvedValue([campaign] as never);
    vi.mocked(publishGet).mockResolvedValue(detail as never);
    vi.mocked(operationListRuns).mockResolvedValue([{
      id: `publish:${campaign.id}`, sourceId: campaign.id, kind: "publish", title: "Đăng bài", state: "uncertain",
      targetCount: 1, totalItems: 1, completedItems: 0, issueCount: 1, retryableCount: 1, retryScope: "linkAndSheet",
      createdAt: campaign.createdAt, updatedAt: campaign.updatedAt,
    }]);
    vi.mocked(publishReconcile).mockResolvedValue({ campaignId: campaign.id, inputDigest: "digest",
      status: "uncertain", retryScope: "linkAndSheet", reportJson: { sheetEnabled: true }, updatedAt: campaign.updatedAt });
    executeCampaign.mockResolvedValueOnce({ campaignId: campaign.id, status: "uncertain", retryScope: "linkAndSheet", issues: [], detail } as never);
    render(<PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />);
    fireEvent.click(screen.getByRole("tab", { name: "Theo dõi" }));
    expect(await screen.findByText("Cần kiểm tra bài đăng")).toBeVisible();
    expect(executeCampaign).not.toHaveBeenCalled();
    expect(screen.queryByRole("button", { name: "Chạy lại từ đầu" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Chi tiết máy" }));
    expect(await screen.findByText(/Hồ sơ có bản nháp; chưa tìm thấy bài đã gửi\./)).toBeVisible();
    expect(screen.getByText(/Tự kiểm tra đã dừng · chọn Kiểm tra liên kết/)).toBeVisible();
    expect(screen.queryByText("Đang chờ xác minh bài đăng")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Kiểm tra liên kết" }));
    await waitFor(() => expect(executeCampaign).toHaveBeenCalledWith(campaign.id, true));
    expect(requestConfirm).toHaveBeenCalledWith(expect.objectContaining({ message: expect.stringContaining("Chỉ tiếp tục lấy liên kết") }));
    expect(createCampaign).not.toHaveBeenCalled();
  });

  it("rejects a stale full-pipeline retry for a publication needing review", async () => {
    const campaign = { id: "review-stale", requestId: "r", sourceRoot: "C:/fixture", state: "uncertain",
      errorCode: "post_verification_needs_review", assignments: [], createdAt: "2026-09-09T00:00:00Z", updatedAt: "2026-09-09T01:00:00Z" };
    vi.mocked(publishList).mockResolvedValue([campaign] as never);
    vi.mocked(operationListRuns).mockResolvedValue([{ id: "publish:review-stale", sourceId: campaign.id, kind: "publish",
      state: "uncertain", retryScope: "linkAndSheet", updatedAt: campaign.updatedAt } as never]);
    vi.mocked(publishReconcile).mockResolvedValue({ campaignId: campaign.id, inputDigest: "digest", status: "uncertain",
      retryScope: "fullPipeline", reportJson: {}, updatedAt: campaign.createdAt });
    render(<PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />);
    fireEvent.click(screen.getByRole("tab", { name: "Theo dõi" }));
    fireEvent.click(await screen.findByRole("button", { name: "Chi tiết máy" }));
    fireEvent.click(await screen.findByRole("button", { name: "Kiểm tra liên kết" }));
    await screen.findByText(/không có bước nào được phép tự chạy lại/);
    expect(executeCampaign).not.toHaveBeenCalled();
    expect(requestConfirm).not.toHaveBeenCalled();
  });

  it("offers submitted posts only the backend-approved link verification scope", async () => {
    vi.mocked(publishGet).mockClear();
    const campaign = {
      id: "pending-link", requestId: "request", sourceRoot: "C:/fixture", state: "verifying",
      visibility: "public", cleanupPolicy: "keepImportedAssets", assignments: [],
      createdAt: "2026-09-09T00:00:00Z", updatedAt: "2026-09-09T00:01:00Z",
    };
    vi.mocked(publishList).mockResolvedValue([campaign] as never);
    vi.mocked(operationListRuns).mockResolvedValue([{
      id: `publish:${campaign.id}`, sourceId: campaign.id, kind: "publish", title: "Đăng bài",
      state: "running", targetCount: 1, totalItems: 1, completedItems: 0, issueCount: 0,
      retryableCount: 1, retryScope: "linkAndSheet", createdAt: campaign.createdAt, updatedAt: campaign.updatedAt,
    }]);
    vi.mocked(publishReconcile).mockResolvedValue({
      campaignId: campaign.id, inputDigest: "digest", status: "partial", retryScope: "linkAndSheet",
      reportJson: { sheetEnabled: false }, updatedAt: campaign.updatedAt,
    });
    render(<PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />);
    fireEvent.click(screen.getByRole("tab", { name: "Theo dõi" }));
    await screen.findByRole("button", { name: "Chi tiết máy" });
    expect(publishReconcile).not.toHaveBeenCalled();
    expect(publishGet).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Chi tiết máy" }));
    expect(await screen.findByRole("button", { name: "Kiểm tra liên kết" })).toBeEnabled();
    fireEvent.click(await screen.findByRole("button", { name: "Kiểm tra liên kết" }));
    await waitFor(() => expect(executeCampaign).toHaveBeenCalledWith(campaign.id, true));
    expect(requestConfirm).toHaveBeenCalledWith(expect.objectContaining({ message: expect.stringContaining("Chỉ tiếp tục lấy liên kết") }));
    expect(createCampaign).not.toHaveBeenCalled();
    expect(screen.queryByRole("button", { name: "Chạy lại từ đầu" })).toBeNull();
  });

  it("explains that a submitted response will wait for a free phone and a verified link", async () => {
    const campaign = {
      id: "queue-pending", requestId: "request", sourceRoot: "C:/fixture", state: "verifying",
      visibility: "public", cleanupPolicy: "keepImportedAssets", assignments: [],
      createdAt: "2026-09-09T00:00:00Z", updatedAt: "2026-09-09T00:01:00Z",
    };
    vi.mocked(publishList).mockResolvedValue([campaign] as never);
    vi.mocked(operationListRuns).mockResolvedValue([{
      id: `publish:${campaign.id}`, sourceId: campaign.id, kind: "publish", title: "Đăng bài", state: "running",
      targetCount: 1, totalItems: 1, completedItems: 0, issueCount: 0, retryableCount: 1,
      retryScope: "linkAndSheet", createdAt: campaign.createdAt, updatedAt: campaign.updatedAt,
    }]);
    vi.mocked(publishReconcile).mockResolvedValue({ campaignId: campaign.id, inputDigest: "digest",
      status: "partial", retryScope: "linkAndSheet", reportJson: { sheetEnabled: true }, updatedAt: campaign.updatedAt });
    executeCampaign.mockResolvedValueOnce({ campaignId: campaign.id, status: "partial", retryScope: "linkAndSheet", issues: [],
      detail: { campaign, bundles: [], events: [], assignments: [{ id: "pending", state: "verifying" }] } } as never);
    render(<PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />);
    fireEvent.click(screen.getByRole("tab", { name: "Theo dõi" }));
    fireEvent.click(await screen.findByRole("button", { name: "Chi tiết máy" }));
    fireEvent.click(await screen.findByRole("button", { name: "Kiểm tra liên kết" }));
    expect(await screen.findByText(/1 bài đã bấm Đăng, đang chờ TikTok hoàn tất/)).toHaveTextContent("Riviu tự kiểm tra khi máy rảnh");
    expect(screen.getByText(/1 bài đã bấm Đăng, đang chờ TikTok hoàn tất/)).toHaveTextContent("Sheet chờ liên kết đã xác minh.");
    expect(createCampaign).not.toHaveBeenCalled();
  });

  it("shows the confirmed post link and account sound evidence separately from Sheet completion", async () => {
    const campaign = {
      id: "posted-evidence",
      requestId: "request",
      sourceRoot: "C:/fixture",
      state: "succeeded",
      visibility: "public",
      cleanupPolicy: "deleteImportedAssetsAfterVerified",
      assignments: [],
      createdAt: "2026-09-05T00:00:00Z",
      updatedAt: "2026-09-05T00:00:00Z",
    };
    vi.mocked(publishList).mockResolvedValueOnce([campaign] as never);
    vi.mocked(publishReconcile).mockResolvedValueOnce({
      campaignId: campaign.id,
      inputDigest: "digest",
      status: "partial",
      retryScope: "sheetOnly",
      reportJson: {},
      updatedAt: campaign.updatedAt,
    });
    vi.mocked(publishGet).mockResolvedValueOnce({
      campaign,
      bundles: [],
      events: [],
      assignments: [
        {
          id: "assignment",
          campaignId: campaign.id,
          bundleId: "bundle",
          ordinal: 0,
          udid: "PHONE-A",
          state: "succeeded",
          sheetDelivery: {
            state: "failed", attempts: 2, lastError: "Google tạm thời gián đoạn",
            nextAttemptAtMs: Date.parse("2026-09-05T00:02:00Z"), updatedAt: "2026-09-05T00:01:00Z",
          },
          evidenceJson: JSON.stringify({
            post: {
              postUrl: "https://www.tiktok.com/@fixture/video/123",
              soundSelection: {
                title: "Bài nhạc trên tài khoản",
                artist: "Tác giả",
                section: "recommended",
                index: 2,
                candidatesDigest: "sound-digest",
                confirmed: true,
              },
            },
            cleanup: { state: "cleaned" },
          }),
        },
      ],
    } as never);
    render(
      <PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />,
    );
    fireEvent.click(screen.getByRole("tab", { name: "Theo dõi" }));
    fireEvent.click(
      await screen.findByRole("button", { name: "Chi tiết máy" }),
    );
    expect(
      await screen.findByRole("link", { name: "Mở bài đã xác nhận" }),
    ).toHaveAttribute("href", "https://www.tiktok.com/@fixture/video/123");
    expect(screen.getByText("Bài nhạc trên tài khoản · Tác giả")).toBeVisible();
    expect(screen.getByText("Sheet chưa hoàn tất")).toBeVisible();
    expect(screen.getByText("Đang chờ ghi Sheet")).toBeVisible();
    expect(screen.getByText("Google tạm thời gián đoạn")).toBeVisible();
    expect(screen.getByText(/Đã thử 2 lần/)).toBeVisible();
    expect(screen.getByText(/Thử tiếp:/)).toBeVisible();
    expect(screen.queryByText("Sheet đã xác nhận")).toBeNull();
    fireEvent.click(screen.getByText("Đã xác nhận nhạc"));
    expect(screen.getByText("sound-digest")).toBeVisible();
    expect(screen.getByText("Đề xuất", { exact: true })).toBeVisible();
  });

  it("distinguishes loading, load failure with retry, and a genuinely empty monitor", async () => {
    const user = userEvent.setup();
    const list = vi.mocked(publishList);
    let rejectFirst!: (reason: Error) => void;
    list.mockImplementationOnce(
      () =>
        new Promise((_, reject) => {
          rejectFirst = reject;
        }),
    );

    render(
      <PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />,
    );
    await user.click(screen.getByRole("tab", { name: "Theo dõi" }));
    expect(screen.getByText("Đang tải chiến dịch…")).toBeVisible();
    expect(screen.queryByText("Chưa có chiến dịch")).toBeNull();

    rejectFirst(new Error("không đọc được chiến dịch"));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "không đọc được chiến dịch",
    );
    expect(screen.queryByText("Chưa có chiến dịch")).toBeNull();

    list.mockResolvedValueOnce([] as never);
    await user.click(screen.getByRole("button", { name: "Thử lại" }));
    expect(await screen.findByText("Chưa có chiến dịch")).toBeVisible();
  });

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

    render(
      <PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />,
    );
    await waitFor(() => expect(list).toHaveBeenCalledTimes(1));

    fire({ type: "publishUpdated", campaignId: "campaign-1", revision: 2 });
    await waitFor(() => expect(list).toHaveBeenCalledTimes(2));
    fire({ type: "publishUpdated", campaignId: "campaign-1", revision: 3 });
    await waitFor(() => expect(list).toHaveBeenCalledTimes(3));

    releaseSucceeded();
    await waitFor(() =>
      expect(screen.getByText("Đã đăng · chờ đối chiếu")).toBeTruthy(),
    );
    releasePosting();

    // The late answer is discarded rather than rendered. Waiting first would pass even
    // without the guard, so this settles the microtask queue and then looks.
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(
      screen.queryByText("Đang đăng"),
      "a reload that started earlier repainted the page over a newer one",
    ).toBeNull();
    expect(screen.getByText("Đã đăng · chờ đối chiếu")).toBeTruthy();

    list.mockReset();
    list.mockResolvedValue([] as never);
    listen.mockReset();
    listen.mockImplementation(async () => () => undefined);
  });

  it("refreshes open machine results on events and ignores older detail replies", async () => {
    let receive: ((event: AppEvent) => void) | undefined;
    vi.mocked(listenRiviuEvents).mockImplementationOnce(async (handler) => {
      receive = handler;
      return () => undefined;
    });
    const campaign = {
      id: "live-publish", requestId: "req-live", sourceRoot: "C:/carousels", state: "posting",
      runAt: null, visibility: "public", cleanupPolicy: "afterPost", assignments: [],
      createdAt: "2026-09-08T00:00:00Z", updatedAt: "2026-09-08T00:00:00Z", errorCode: null,
    };
    const detail = (state: string) => ({ campaign, bundles: [], events: [], assignments: [{
      id: "a-live", campaignId: campaign.id, bundleId: "b1", ordinal: 0, udid: "PHONE-A", state, errorCode: null,
    }] });
    vi.mocked(publishList).mockResolvedValue([campaign] as never);
    vi.mocked(publishGet).mockReset().mockResolvedValueOnce(detail("posting") as never);
    render(<PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />);
    fireEvent.click(screen.getByRole("tab", { name: "Theo dõi" }));
    fireEvent.click(await screen.findByRole("button", { name: "Chi tiết máy" }));
    const panel = await screen.findByRole("region", { name: "Chi tiết chiến dịch đang chọn" });
    await waitFor(() => expect(within(panel).getByText("Đang đăng")).toBeVisible());
    let older!: (value: unknown) => void;
    vi.mocked(publishGet).mockImplementationOnce(() => new Promise(resolve => { older = resolve as (value: unknown) => void; }));
    vi.mocked(publishGet).mockResolvedValueOnce(detail("succeeded") as never);
    await act(async () => receive!({ type: "publishUpdated", campaignId: campaign.id, revision: 2 }));
    await waitFor(() => expect(publishGet).toHaveBeenCalledTimes(2));
    await act(async () => receive!({ type: "publishUpdated", campaignId: campaign.id, revision: 3 }));
    await waitFor(() => expect(within(panel).queryByText("Đang đăng")).toBeNull());
    await act(async () => older(detail("verifying")));
    expect(within(panel).queryByText("Đã bấm Đăng · chờ xác minh")).toBeNull();
    expect(within(panel).getByText("Đã đăng")).toBeVisible();
    expect(publishReconcile).toHaveBeenCalledTimes(1);
    expect(executeCampaign).not.toHaveBeenCalled();
  });
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

    render(
      <PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />,
    );

    await user.click(screen.getByRole("tab", { name: "Theo dõi" }));
    await user.click(await screen.findByRole("button", { name: "Chi tiết máy" }));
    const retry = await screen.findByRole("button", {
      name: "Chạy lại từ đầu",
    });
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
    list.mockResolvedValue([
      {
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
      },
    ] as never);
    vi.mocked(publishReconcile).mockResolvedValue({
      campaignId: "campaign-locked",
      inputDigest: "approved-digest-1",
      status: "uncertain",
      retryScope: "none",
      reportJson: {},
      updatedAt: "2026-09-04T00:00:00Z",
    });

    render(
      <PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />,
    );
    await user.click(screen.getByRole("tab", { name: "Theo dõi" }));
    await user.click(await screen.findByRole("button", { name: "Chi tiết máy" }));
    await user.click(
      await screen.findByRole("button", { name: "Chạy lại từ đầu" }),
    );

    expect(
      await screen.findByText(/không có bước nào được phép tự chạy lại/i),
    ).toBeVisible();
    expect(executeCampaign).not.toHaveBeenCalled();
    expect(requestConfirm).not.toHaveBeenCalled();

    list.mockReset();
    list.mockResolvedValue([] as never);
  });

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

    const namedDevices = [
      { ...devices[0], name: "SM-G950F" },
      ...devices.slice(1),
    ];
    const metas = new Map<string, DeviceMeta>([
      [
        "PHONE-A",
        {
          udid: "PHONE-A",
          notes: "",
          tags: [],
          alias: "Máy quay sản phẩm",
          number: 17,
        },
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
    await user.click(
      await screen.findByRole("button", { name: "Chi tiết máy" }),
    );
    expect(publishReconcile).toHaveBeenCalledWith("campaign-9");
    expect(
      await screen.findByText("Chỉ tiếp tục lấy liên kết và ghi Sheet"),
    ).toBeVisible();
    const technical = await screen.findByRole("group", {
      name: "Chi tiết kỹ thuật máy",
    });
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
    expect(
      screen.getByText(/chưa dọn được ảnh tạm: adb disconnected/),
    ).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Ẩn chi tiết máy" }));
    expect(screen.queryByText(/route_authorities_disagree/)).toBeNull();

    list.mockReset();
    list.mockResolvedValue([] as never);
  });
  it("keeps a closed detail pane empty after switching campaigns and receiving a late response", async () => {
    const campaigns = ["first", "second"].map(id => ({ id, requestId: id, sourceRoot: `C:/${id}`,
      state: "failedBeforeDispatch", assignments: [], createdAt: "2026-09-09T00:00:00Z", updatedAt: "2026-09-09T00:00:00Z" }));
    const detail = (index: number) => ({ campaign: campaigns[index], bundles: [], events: [], assignments: [] });
    let releaseSecond!: (value: unknown) => void;
    vi.mocked(publishList).mockResolvedValue(campaigns as never);
    vi.mocked(publishGet).mockReset().mockResolvedValueOnce(detail(0) as never)
      .mockImplementationOnce(() => new Promise(resolve => { releaseSecond = resolve as (value: unknown) => void; }));
    render(<PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />);
    await userEvent.click(screen.getByRole("tab", { name: "Theo dõi" }));
    const list = await screen.findByRole("list", { name: "Chiến dịch đăng bài" });
    const rows = within(list).getAllByRole("listitem");
    await userEvent.click(within(rows[0]).getByRole("button", { name: "Chi tiết máy" }));
    await screen.findByRole("region", { name: "Chi tiết chiến dịch đang chọn" });
    await userEvent.click(within(rows[1]).getByRole("button", { name: "Chi tiết máy" }));
    await waitFor(() => expect(publishGet).toHaveBeenCalledWith("second"));
    await userEvent.click(screen.getByRole("button", { name: "Ẩn chi tiết máy" }));
    expect(screen.getByText("Chọn một chiến dịch để theo dõi")).toBeVisible();
    await act(async () => releaseSecond(detail(1)));
    expect(screen.getByText("Chọn một chiến dịch để theo dõi")).toBeVisible();
    expect(screen.queryByRole("region", { name: "Chi tiết chiến dịch đang chọn" })).toBeNull();
    expect(createCampaign).not.toHaveBeenCalled();
    expect(executeCampaign).not.toHaveBeenCalled();
  });
  it("hides selected campaign actions when a monitor filter excludes that campaign", async () => {
    const campaign = { id: "hidden-by-filter", requestId: "r", sourceRoot: "C:/needs-review", state: "failedBeforeDispatch",
      assignments: [], createdAt: "2026-09-09T00:00:00Z", updatedAt: "2026-09-09T00:00:00Z" };
    vi.mocked(publishList).mockResolvedValue([campaign] as never);
    vi.mocked(publishGet).mockResolvedValue({ campaign, bundles: [], events: [], assignments: [] } as never);
    render(<PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />);
    await userEvent.click(screen.getByRole("tab", { name: "Theo dõi" }));
    await userEvent.click(await screen.findByRole("button", { name: "Chi tiết máy" }));
    await screen.findByRole("button", { name: "Chạy lại từ đầu" });
    await userEvent.click(within(screen.getByRole("group", { name: "Lọc chiến dịch" })).getByRole("button", { name: /^Hoàn tất/ }));
    expect(screen.queryByRole("button", { name: "Chạy lại từ đầu" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Ẩn chi tiết máy" })).toBeNull();
    expect(screen.getByText("Chọn một chiến dịch để theo dõi")).toBeVisible();
    expect(executeCampaign).not.toHaveBeenCalled();
  });
});

it("quick selection trims to available capacity and keeps pairs across an offline roster", async () => {
  const props = { devices: devices.slice(0, 2), selected: [], onSelectUdids: () => {}, targetRef: { type: "all" as const }, targetUdids: ["PHONE-A", "PHONE-B"] };
  const view = render(<PublishPage {...props}/>);
  await userEvent.click(screen.getByRole("button", { name: "Chọn thư mục" }));
  await userEvent.click(screen.getByRole("button", { name: "Quét" }));
  await screen.findByRole("checkbox", { name: "Chọn bo1" });
  await userEvent.click(screen.getByRole("button", { name: "Chọn nhanh" }));
  const check = screen.getByRole("button", { name: "Kiểm tra & đăng" });
  await waitFor(() => expect(check).toBeEnabled());
  expect(screen.getByRole("checkbox", { name: "Chọn bo1" })).toBeChecked();
  expect(screen.getByRole("checkbox", { name: "Chọn bo2" })).toBeChecked();
  expect(screen.getByRole("checkbox", { name: "Chọn bo3" })).not.toBeChecked();
  view.rerender(<PublishPage {...props} devices={[devices[0]]} targetUdids={["PHONE-A"]}/>);
  await waitFor(() => expect(check).toBeDisabled());
  expect(screen.getByRole("checkbox", { name: "Chọn bo2" })).toBeChecked();
  view.rerender(<PublishPage {...props}/>);
  await waitFor(() => expect(check).toBeEnabled());
  await userEvent.click(check);
  await waitFor(() => expect(preflightCampaign).toHaveBeenCalledWith(expect.objectContaining({ bundleIds: ["b1", "b2"], udids: ["PHONE-A", "PHONE-B"] })));
});
