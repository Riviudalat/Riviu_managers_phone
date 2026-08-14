import { beforeEach, describe, expect, it, vi } from "vitest";
import type { DeviceInfo } from "./types";
import { startDevicePreview, startFleetPreview } from "./startPreview";

vi.mock("./api", () => ({
  prepareDevice: vi.fn(async () => undefined),
  viewEnsure: vi.fn(async () => undefined),
}));

const android: DeviceInfo = {
  udid: "10969614",
  name: "Redmi",
  model: "23021RAAEG",
  platform: "android",
  osVersion: "15",
  connection: "usb",
  status: "ready",
  wdaReady: false,
};

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

describe("startDevicePreview", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("uses viewEnsure on Android and never prepareDevice", async () => {
    const api = await import("./api");
    await startDevicePreview(android);
    expect(api.viewEnsure).toHaveBeenCalledWith("10969614");
    expect(api.prepareDevice).not.toHaveBeenCalled();
  });

  it("uses prepareDevice on iPhone and never viewEnsure", async () => {
    const api = await import("./api");
    await startDevicePreview(iphone);
    expect(api.prepareDevice).toHaveBeenCalledWith(iphone.udid);
    expect(api.viewEnsure).not.toHaveBeenCalled();
  });
});

describe("startFleetPreview", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("starts every Android phone and does not send them through prepareDevice", async () => {
    const api = await import("./api");
    const second: DeviceInfo = { ...android, udid: "ce06171646f0d7e", name: "Note 8" };
    await startFleetPreview([android, iphone, second]);
    expect(api.viewEnsure).toHaveBeenCalledWith("10969614");
    expect(api.viewEnsure).toHaveBeenCalledWith("ce06171646f0d7e");
    expect(api.prepareDevice).toHaveBeenCalledTimes(1);
    expect(api.prepareDevice).toHaveBeenCalledWith(iphone.udid);
  });
});
