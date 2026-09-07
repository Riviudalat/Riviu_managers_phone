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
import { useState } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  automationGet,
  automationList,
  listenRiviuEvents,
  operationListRuns,
  publishGet,
  publishList,
  publishReconcile,
  publishScanFolder,
  publishSheetGetConfig,
  publishSheetSaveConfig,
} from "../api";
import { PublishPage } from "./PublishPage";
import { requestConfirm } from "../confirmStore";
import { requestWorkspaceLeave } from "../workspaceDraft";
import { resetToasts } from "../toastStore";
import { pickDirectory } from "../pickFile";
import type {
  AppEvent,
  DeviceInfo,
  DeviceMeta,
  PublishBundle,
  PublishFolderManifest,
  PublishPreflightReport,
  PublishPreflightRequest,
  TargetRef,
  AutomationDefinitionRecord,
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
  publishSheetSaveConfig: vi.fn(async () => ({
    webhookUrl: "",
    hasToken: false,
  })),
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
  vi.mocked(automationList).mockReset().mockResolvedValue([]);
  vi.mocked(automationGet).mockReset();
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
  vi.mocked(publishSheetSaveConfig)
    .mockReset()
    .mockResolvedValue({ webhookUrl: "", hasToken: false });
  vi.mocked(publishScanFolder).mockReset().mockResolvedValue(manifest);
  vi.mocked(pickDirectory).mockReset().mockResolvedValue("C:/carousels");
  resetToasts();
});

afterEach(cleanup);

async function prepareOne() {
  await userEvent.click(screen.getByRole("button", { name: "Chọn thư mục" }));
  await userEvent.click(
    await screen.findByRole("checkbox", { name: "Chọn bo1" }),
  );
  await userEvent.click(screen.getByRole("button", { name: "Chọn máy" }));
  await userEvent.click(screen.getByRole("button", { name: /Ghép tự động/ }));
  await userEvent.click(
    screen.getByRole("button", { name: "Xem lại & kiểm tra" }),
  );
}

describe("production publish wizard", () => {
  it("autosaves mapping and caption, rescans on remount and requires fresh preflight", async () => {
    const view = render(<PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />);
    await prepareOne();
    await userEvent.click(screen.getByRole("checkbox", { name: "Xóa ảnh đã chuyển trên máy" }));
    await act(async () => { expect(await requestWorkspaceLeave()).toBe(true); });
    expect(requestConfirm).not.toHaveBeenCalled();
    expect(createCampaign).not.toHaveBeenCalled();
    view.unmount();
    render(<PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />);
    await waitFor(() => expect(screen.getByRole("checkbox", { name: "Chọn bo1" })).toBeChecked());
    expect(publishScanFolder).toHaveBeenCalledTimes(2);
    await userEvent.click(screen.getByRole("button", { name: "Chọn máy" }));
    expect(screen.getByRole("button", { name: "Máy 1: bo1" })).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: "Xem lại & kiểm tra" }));
    expect(screen.getByRole("checkbox", { name: "Xóa ảnh đã chuyển trên máy" })).toBeChecked();
    expect(preflightCampaign).not.toHaveBeenCalled();
    expect(executeCampaign).not.toHaveBeenCalled();
  });
  it("scans real source but never selects or dispatches on load", async () => {
    render(
      <PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Chọn thư mục" }));
    expect(
      await screen.findByRole("checkbox", { name: "Chọn bo1" }),
    ).not.toBeChecked();
    expect(screen.getByRole("button", { name: "Chọn máy" })).toBeDisabled();
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
    await userEvent.click(screen.getByRole("button", { name: "Quét nguồn" }));
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
      screen.getByRole("checkbox", { name: "Xóa ảnh đã chuyển trên máy" }),
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Kiểm tra 1 bài" }),
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
      screen.getByRole("button", { name: "Kiểm tra 1 bài" }),
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
      screen.getByRole("button", { name: "Kiểm tra 1 bài" }),
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
      screen.getByRole("button", { name: "Kiểm tra 1 bài" }),
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
  it("applies a group profile against its newly resolved machines and restores cleanup", async () => {
    const record = {
      definition: {
        id: "profile-b",
        name: "Nhóm B",
        kind: "publish",
        latestRevision: 1,
        archived: false,
        createdAt: "2026-09-07T00:00:00Z",
        updatedAt: "2026-09-07T00:00:00Z",
      },
      revision: {
        definitionId: "profile-b",
        revision: 1,
        createdAt: "2026-09-07T00:00:00Z",
        canonicalJson: "{}",
        sha256: "ab".repeat(32),
        targetRef: { type: "group", groupId: "b" },
        config: {
          schemaVersion: 1,
          sourceRoot: "C:/carousels",
          bundleIds: ["b1"],
          captionOverrides: { b1: "Caption B" },
          soundPolicy: { kind: "trendingAny", poolSize: 5, seed: 42 },
          executionConfirmed: true,
          sheetEnabled: false,
          deleteAfterPublish: false,
        },
      },
    } as AutomationDefinitionRecord;
    vi.mocked(automationList).mockResolvedValue([record.definition]);
    vi.mocked(automationGet).mockResolvedValue(record);
    function Workspace() {
      const [target, setTarget] = useState<TargetRef>({
        type: "group",
        groupId: "a",
      });
      return (
        <PublishPage
          devices={devices}
          selected={[]}
          targetRef={target}
          onTargetRefChange={setTarget}
          targetUdids={
            target.type === "group" && target.groupId === "b"
              ? ["PHONE-B"]
              : ["PHONE-A"]
          }
          onSelectUdids={() => {}}
        />
      );
    }
    render(<Workspace />);
    await userEvent.click(
      screen.getByRole("button", { name: "Hồ sơ & cài đặt" }),
    );
    await userEvent.selectOptions(
      await screen.findByRole("combobox", { name: "Hồ sơ Đăng bài" }),
      "profile-b",
    );
    await waitFor(() => expect(publishScanFolder).toHaveBeenCalled());
    await userEvent.click(screen.getByRole("button", { name: "Đóng" }));
    await userEvent.click(screen.getByRole("button", { name: "Chọn máy" }));
    expect(screen.getByRole("button", { name: "Máy 2: bo1" })).toBeVisible();
    await userEvent.click(
      screen.getByRole("button", { name: "Xem lại & kiểm tra" }),
    );
    expect(
      screen.getByRole("checkbox", { name: "Xóa ảnh đã chuyển trên máy" }),
    ).not.toBeChecked();
    await userEvent.click(
      screen.getByRole("button", { name: "Kiểm tra 1 bài" }),
    );
    expect(preflightCampaign).toHaveBeenLastCalledWith(
      expect.objectContaining({
        udids: ["PHONE-B"],
        soundPolicy: { kind: "trendingAny", poolSize: 5, seed: 42 },
        deleteAfterPublish: false,
      }),
    );
  });
  it("does not publish when native confirmation is cancelled", async () => {
    vi.mocked(requestConfirm).mockResolvedValueOnce(false);
    render(
      <PublishPage devices={devices} selected={[]} onSelectUdids={() => {}} />,
    );
    await prepareOne();
    await userEvent.click(
      screen.getByRole("button", { name: "Kiểm tra 1 bài" }),
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
      screen.getByRole("button", { name: "Kiểm tra 1 bài" }),
    );
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Xác nhận đăng 1 bài" }),
      ).toBeEnabled(),
    );
    await userEvent.click(screen.getByRole("button", { name: "Đóng" }));
    await userEvent.click(
      screen.getByRole("checkbox", { name: "Xóa ảnh đã chuyển trên máy" }),
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Kiểm tra 1 bài" }),
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
    await userEvent.click(
      await screen.findByRole("checkbox", { name: "Chọn bo1" }),
    );
    await userEvent.click(screen.getByRole("checkbox", { name: "Chọn bo2" }));
    await userEvent.click(screen.getByRole("button", { name: "Chọn máy" }));
    await userEvent.click(screen.getByRole("button", { name: /Ghép tự động/ }));
    await userEvent.click(screen.getByRole("button", { name: "Quay lại" }));
    await userEvent.click(screen.getByRole("checkbox", { name: "Chọn bo1" }));
    await userEvent.click(screen.getByRole("button", { name: "Chọn máy" }));
    await userEvent.click(
      screen.getByRole("button", { name: "Xem lại & kiểm tra" }),
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Kiểm tra 1 bài" }),
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
    await userEvent.click(
      await screen.findByRole("button", { name: "Sửa nội dung bo1" }),
    );
    fireEvent.change(
      screen.getByRole("textbox", { name: "Chú thích cho bo1" }),
      { target: { value: "Nội dung mới" } },
    );
    await userEvent.click(screen.getByRole("button", { name: "Lưu nội dung" }));
    expect(manifest.bundles[0].caption).toBe("caption for bo1");
    await userEvent.click(screen.getByRole("checkbox", { name: "Chọn bo1" }));
    await userEvent.click(screen.getByRole("button", { name: "Chọn máy" }));
    await userEvent.click(screen.getByRole("button", { name: /Ghép tự động/ }));
    await userEvent.click(
      screen.getByRole("button", { name: "Xem lại & kiểm tra" }),
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Kiểm tra 1 bài" }),
    );
    expect(preflightCampaign).toHaveBeenLastCalledWith(
      expect.objectContaining({ captionOverrides: { b1: "Nội dung mới" } }),
    );
  });
});

describe("publish campaign monitoring", () => {
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
    expect(screen.getByRole("region", { name: "Theo dõi" })).toBeVisible();
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
    vi.mocked(publishReconcile).mockResolvedValueOnce({
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
    fireEvent.click(screen.getByRole("button", { name: "Theo dõi" }));
    expect(await screen.findByText("Hoàn tất một phần")).toBeVisible();
    expect(screen.queryByText("Hoàn tất", { exact: true })).toBeNull();
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
    fireEvent.click(screen.getByRole("button", { name: "Theo dõi" }));
    fireEvent.click(
      await screen.findByRole("button", { name: "Đối chiếu kết quả" }),
    );
    expect(
      await screen.findByRole("link", { name: "Mở bài đã xác nhận" }),
    ).toHaveAttribute("href", "https://www.tiktok.com/@fixture/video/123");
    expect(screen.getByText("Bài nhạc trên tài khoản · Tác giả")).toBeVisible();
    expect(screen.getByText("Sheet chưa hoàn tất")).toBeVisible();
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
    await user.click(screen.getByRole("button", { name: "Theo dõi" }));
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

    await user.click(screen.getByRole("button", { name: "Theo dõi" }));
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
    vi.mocked(publishReconcile).mockResolvedValueOnce({
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
    await user.click(screen.getByRole("button", { name: "Theo dõi" }));
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

    await user.click(screen.getByRole("button", { name: "Theo dõi" }));
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
});
