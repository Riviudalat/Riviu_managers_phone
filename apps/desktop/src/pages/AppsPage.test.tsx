import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { AppsPage } from "./FarmPages";
import { resetToasts } from "../toastStore";
import type { AppLibraryItem, DeviceInfo } from "../types";

const library: AppLibraryItem[] = [
  {
    id: "app-1",
    name: "TikTok.ipa",
    path: "C:/ipa/tiktok.ipa",
    bundleId: "com.ss.iphone.ugc.Ame",
    version: "35.0.0",
    createdAt: "2026-08-17T00:00:00.000Z",
  },
];

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
  pickDirectory: vi.fn(async () => null),
  pickIpa: vi.fn(async () => null),
  pickMaterial: vi.fn(async () => null),
}));

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
beforeEach(() => vi.clearAllMocks());

function renderApps(devices: DeviceInfo[], selected: string[] = []) {
  return render(
    <AppsPage devices={devices} selected={selected} onSelectUdids={() => undefined} />,
  );
}

describe("AppsPage install targets", () => {
  it("never counts an Android phone as a target for an IPA", async () => {
    // `targetsOf` falls back to *every connected device* when nothing is selected, so an
    // unselected click pushed an iOS app at every Android serial in the room and collected
    // one failure per phone. On this fleet that is twenty guaranteed failures from a button
    // whose label said it was about to install on twenty machines.
    renderApps([iphone, android, { ...android, udid: "ce0617", name: "Note 8" }]);

    expect(await screen.findByRole("button", { name: "Install → 1 iPhone" })).toBeEnabled();
  });

  it("says which selected phones it is leaving out, rather than dropping them silently", async () => {
    renderApps([iphone, android], [iphone.udid, android.udid]);

    expect(await screen.findByText(/Bỏ qua 1 máy Android đang chọn/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Install → 1 iPhone" })).toBeEnabled();
  });

  it("disables the install on an Android-only fleet and explains why", async () => {
    renderApps([android]);

    const button = await screen.findByRole("button", { name: "Install → 0 iPhone" });
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

    await userEvent.click(await screen.findByRole("button", { name: "Install → 2 iPhone" }));

    await waitFor(() => expect(api.installLibraryApp).toHaveBeenCalledTimes(2));
    expect(api.installLibraryApp).toHaveBeenCalledWith(iphone.udid, "app-1");
    expect(api.installLibraryApp).toHaveBeenCalledWith("second-iphone", "app-1");
  });
});
