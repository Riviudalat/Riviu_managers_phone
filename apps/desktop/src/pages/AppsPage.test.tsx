import { StrictMode } from "react";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { AppsPage } from "./AppsPage";
import { listAppsLibrary, listGroups } from "../api";
import { requestConfirm } from "../confirmStore";
import { resetToasts } from "../toastStore";
import type { AppLibraryItem, DeviceInfo } from "../types";

const library: AppLibraryItem[] = [
  {
    id: "app-1",
    name: "TikTok.ipa",
    path: "C:/ipa/tiktok.ipa",
    bundleId: "com.ss.iphone.ugc.Ame",
    version: "35.0.0",
    platform: "ios",
    packageFormat: "ipa",
    createdAt: "2026-08-17T00:00:00.000Z",
  },
];

const androidLibrary: AppLibraryItem[] = [
  {
    id: "android-app-1",
    name: "TikTok.apkm",
    path: "C:/apk/tiktok.apkm",
    bundleId: "com.zhiliaoapp.musically",
    version: "36.0.0",
    platform: "android",
    packageFormat: "apkm",
    createdAt: "2026-09-02T00:00:00.000Z",
  },
];

vi.mock("../api", () => ({
  addAppLibrary: vi.fn(async () => undefined),
  cancelAppInstallBatch: vi.fn(async () => undefined),
  addMaterial: vi.fn(async () => undefined),
  analyticsSummary: vi.fn(async () => ({})),
  apiDocs: vi.fn(async () => ""),
  deleteAppLibrary: vi.fn(async () => undefined),
  deleteMaterial: vi.fn(async () => undefined),
  deleteSchedule: vi.fn(async () => undefined),
  exampleScript: vi.fn(async () => "{}"),
  installLibraryAppBatch: vi.fn(async (request: { batchId: string; udids: string[] }) => ({
    batchId: request.batchId,
    progress: [],
    results: request.udids.map((udid) => ({
      udid,
      status: "succeeded" as const,
      effectStarted: true,
    })),
  })),
  listAppsLibrary: vi.fn(async () => library),
  listGroups: vi.fn(async () => []),
  listMaterials: vi.fn(async () => []),
  listSchedules: vi.fn(async () => []),
  listScripts: vi.fn(async () => []),
  publishCancel: vi.fn(async () => undefined),
  publishCreateCampaign: vi.fn(async () => ({})),
  publishList: vi.fn(async () => []),
  publishPrepare: vi.fn(async () => undefined),
  publishPost: vi.fn(async () => undefined),
  publishScanFolder: vi.fn(async () => ({})),
  publishTransfer: vi.fn(async () => undefined),
  pushMaterial: vi.fn(async () => undefined),
  saveSchedule: vi.fn(async () => undefined),
  saveScript: vi.fn(async () => undefined),
}));

vi.mock("../pickFile", () => ({
  pickFile: vi.fn(async () => null),
  pickDirectory: vi.fn(async () => null),
  pickIpa: vi.fn(async () => null),
  pickMaterial: vi.fn(async () => null),
}));

vi.mock("../confirmStore", () => ({ requestConfirm: vi.fn(async () => true) }));

const iphone: DeviceInfo = {
  udid: "a99f4bd9f877b2a0e3682ee24fd1c68f75ba6982",
  name: "iPhone 8",
  model: "iPhone10,1",
  platform: "ios",
  osVersion: "16.7.15",
  connection: "usb",
  status: "ready",
  wdaReady: true,
};

const android: DeviceInfo = {
  udid: "ce051715081fe20f03",
  name: "Galaxy S8+",
  model: "SM-G955F",
  platform: "android",
  osVersion: "9",
  connection: "usb",
  status: "ready",
  wdaReady: false,
};

afterEach(() => {
  cleanup();
  resetToasts();
});
beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(requestConfirm).mockResolvedValue(true);
  vi.mocked(listAppsLibrary).mockResolvedValue(library);
  vi.mocked(listGroups).mockResolvedValue([]);
});

function renderApps(devices: DeviceInfo[], selected: string[] = []) {
  return render(
    <AppsPage devices={devices} selected={selected} onSelectUdids={() => undefined} />,
  );
}

describe("AppsPage install targets", () => {
  it("routes an Android split package only to selected Android devices", async () => {
    const api = await import("../api");
    vi.mocked(listAppsLibrary).mockResolvedValue(androidLibrary);
    renderApps([iphone, android], [iphone.udid, android.udid]);

    await userEvent.click(await screen.findByRole("button", { name: "Cài → 1 Android" }));

    expect(api.installLibraryAppBatch).toHaveBeenCalledOnce();
    expect(api.installLibraryAppBatch).toHaveBeenCalledWith(expect.objectContaining({
      appId: "android-app-1",
      udids: [android.udid],
      allowDowngrade: false,
    }));
  });
  it("never counts an Android phone as a target for an IPA", async () => {
    // `targetsOf` falls back to *every connected device* when nothing is selected, so an
    // unselected click pushed an iOS app at every Android serial in the room and collected
    // one failure per phone. On this fleet that is twenty guaranteed failures from a button
    // whose label said it was about to install on twenty machines.
    renderApps([iphone, android, { ...android, udid: "ce0617", name: "Note 8" }]);

    expect(await screen.findByRole("button", { name: "Cài → 1 iPhone" })).toBeEnabled();
  });

  it("keeps platform filtering in the command label without a persistent explanatory note", async () => {
    renderApps([iphone, android], [iphone.udid, android.udid]);

    expect(await screen.findByRole("button", { name: "Cài → 1 iPhone" })).toBeEnabled();
    expect(screen.queryByText(/Bỏ qua .* Android/)).toBeNull();
  });

  it("disables the install on an Android-only fleet and explains why", async () => {
    renderApps([android]);

    const button = await screen.findByRole("button", { name: "Cài → 0 iPhone" });
    expect(button).toBeDisabled();
    expect(button).toHaveAttribute(
      "title",
      expect.stringContaining("IPA chỉ cài được lên iOS"),
    );
  });

  it("installs on every selected iPhone and nothing else", async () => {
    const api = await import("../api");
    const second = { ...iphone, udid: "second-iphone", name: "iPhone 8 (2)" };
    renderApps([iphone, android, second]);

    await userEvent.click(await screen.findByRole("button", { name: "Cài → 2 iPhone" }));

    await waitFor(() => expect(api.installLibraryAppBatch).toHaveBeenCalledOnce());
    expect(api.installLibraryAppBatch).toHaveBeenCalledWith(expect.objectContaining({
      appId: "app-1",
      udids: [iphone.udid, "second-iphone"],
    }));
  });

  it("requires explicit confirmation before requesting downgrade and renders typed outcomes", async () => {
    const api = await import("../api");
    vi.mocked(api.installLibraryAppBatch).mockResolvedValueOnce({
      batchId: "batch-1",
      progress: [],
      results: [{
        udid: iphone.udid,
        status: "uncertain",
        effectStarted: true,
        detail: "readback timed out",
      }],
    });
    renderApps([iphone]);

    await userEvent.click(await screen.findByRole("checkbox", { name: "Cho phép hạ phiên bản" }));
    await userEvent.click(screen.getByRole("button", { name: "Cài → 1 iPhone" }));

    expect(requestConfirm).toHaveBeenCalledWith({
      title: "Cho phép hạ phiên bản?",
      message: expect.stringContaining("1 thiết bị"),
      confirmLabel: "Tiếp tục cài",
      danger: true,
    });
    expect(api.installLibraryAppBatch).toHaveBeenCalledWith(expect.objectContaining({
      allowDowngrade: true,
    }));
    expect(await screen.findByText("Cần kiểm lại")).toBeVisible();
    expect(screen.getByText("readback timed out")).toBeVisible();
  });

  it("keeps installation untouched when downgrade confirmation is declined", async () => {
    const api = await import("../api");
    vi.mocked(requestConfirm).mockResolvedValueOnce(false);
    renderApps([iphone]);

    await userEvent.click(await screen.findByRole("checkbox", { name: "Cho phép hạ phiên bản" }));
    await userEvent.click(screen.getByRole("button", { name: "Cài → 1 iPhone" }));

    expect(requestConfirm).toHaveBeenCalledOnce();
    expect(api.installLibraryAppBatch).not.toHaveBeenCalled();
  });

  it("cancels only the active batch while the backend owns in-flight installs", async () => {
    const api = await import("../api");
    let finish!: (value: Awaited<ReturnType<typeof api.installLibraryAppBatch>>) => void;
    vi.mocked(api.installLibraryAppBatch).mockReturnValueOnce(new Promise((resolve) => {
      finish = resolve;
    }));
    renderApps([iphone]);

    await userEvent.click(await screen.findByRole("button", { name: "Cài → 1 iPhone" }));
    const cancel = await screen.findByRole("button", { name: "Hủy máy chưa bắt đầu" });
    await userEvent.click(cancel);

    const request = vi.mocked(api.installLibraryAppBatch).mock.calls[0][0];
    expect(api.cancelAppInstallBatch).toHaveBeenCalledWith(request.batchId);
    finish({ batchId: request.batchId, progress: [], results: [] });
    await waitFor(() => expect(cancel).not.toBeInTheDocument());
  });
});

describe("AppsPage list states", () => {
  it("does not render an empty library before its first answer", async () => {
    renderApps([iphone]);

    expect(screen.getByText("Đang tải thư viện ứng dụng…")).toBeInTheDocument();
    expect(screen.queryByText("Chưa có ứng dụng")).toBeNull();
    expect(screen.getAllByRole("heading", { level: 2 })).toHaveLength(3);
    expect(await screen.findByText("TikTok.ipa")).toBeInTheDocument();
  });

  it("shows a failed library load inline and retries it", async () => {
    vi.mocked(listAppsLibrary)
      .mockRejectedValueOnce(new Error("Không đọc được thư viện ứng dụng"))
      .mockResolvedValueOnce(library);

    renderApps([iphone]);

    expect(await screen.findByRole("alert")).toHaveTextContent("Không đọc được thư viện ứng dụng");
    expect(screen.queryByText("Chưa có ứng dụng")).toBeNull();

    await userEvent.click(screen.getByRole("button", { name: "Thử lại thư viện ứng dụng" }));

    await waitFor(() => expect(listAppsLibrary).toHaveBeenCalledTimes(2));
    expect(await screen.findByText("TikTok.ipa")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("reports group loading, empty, and error independently from the IPA library", async () => {
    vi.mocked(listAppsLibrary).mockRejectedValueOnce(new Error("library down"));
    vi.mocked(listGroups).mockRejectedValue(new Error("groups down"));

    renderApps([iphone]);

    expect(screen.getByText("Đang tải danh sách nhóm…")).toBeInTheDocument();
    expect(await screen.findByText(/Không tải được danh sách nhóm: groups down/)).toBeInTheDocument();
    expect(screen.getByText(/Không tải được thư viện ứng dụng: library down/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Thử lại danh sách nhóm" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Thử lại thư viện ứng dụng" })).toBeEnabled();
    expect(screen.queryByText("Chưa có nhóm thiết bị")).toBeNull();
    expect(screen.getByText("Danh sách nhóm chưa tải được")).toBeInTheDocument();

    const callsBeforeRetry = vi.mocked(listGroups).mock.calls.length;
    vi.mocked(listGroups).mockResolvedValue([]);
    await userEvent.click(screen.getByRole("button", { name: "Thử lại danh sách nhóm" }));
    await waitFor(() => expect(listGroups).toHaveBeenCalledTimes(callsBeforeRetry + 1));
    expect(await screen.findByText("Chưa có nhóm thiết bị")).toBeInTheDocument();
    expect(screen.queryByText(/Không tải được danh sách nhóm/)).toBeNull();
  });

  it("keeps the newest IPA list when StrictMode responses arrive out of order", async () => {
    let resolveFirst!: (value: AppLibraryItem[]) => void;
    let resolveSecond!: (value: AppLibraryItem[]) => void;
    vi.mocked(listAppsLibrary)
      .mockReturnValueOnce(new Promise((resolve) => { resolveFirst = resolve; }))
      .mockReturnValueOnce(new Promise((resolve) => { resolveSecond = resolve; }));

    render(
      <StrictMode>
        <AppsPage devices={[iphone]} selected={[]} onSelectUdids={() => undefined} />
      </StrictMode>,
    );
    await waitFor(() => expect(listAppsLibrary).toHaveBeenCalledTimes(2));
    resolveSecond(library);
    expect(await screen.findByText("TikTok.ipa")).toBeInTheDocument();
    resolveFirst([{ ...library[0], id: "old", name: "Old.ipa" }]);
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(screen.getByText("TikTok.ipa")).toBeInTheDocument();
    expect(screen.queryByText("Old.ipa")).toBeNull();
  });
});
