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

  it("still prepares every iPhone when an Android phone refuses", async () => {
    // The complaint in one line: this was `Promise.all` over the Android half followed by
    // the iOS loop, so one unplugged Android rejected the whole call and every iPhone
    // behind it was never reached. On a mixed fleet, pressing Start did nothing at all for
    // the iPhones -- and said so with a single error naming only the Android.
    const api = await import("./api");
    vi.mocked(api.viewEnsure).mockRejectedValueOnce(new Error("device offline"));
    const healthy: DeviceInfo = { ...android, udid: "ce06171646f0d7e", name: "Note 8" };

    const failures = await startFleetPreview([android, healthy, iphone]);

    expect(api.prepareDevice).toHaveBeenCalledWith(iphone.udid);
    expect(api.viewEnsure).toHaveBeenCalledWith("ce06171646f0d7e");
    expect(failures).toHaveLength(1);
    expect(failures[0].udid).toBe(android.udid);
    expect(failures[0].name).toBe("Redmi");
  });

  it("keeps going down the iPhone queue after one of them fails", async () => {
    // The sequential half had the same shape: the loop had no guard, so the first iPhone
    // to throw ended the turn of every iPhone queued behind it.
    const api = await import("./api");
    vi.mocked(api.prepareDevice).mockRejectedValueOnce(new Error("WDA is not running"));
    const second: DeviceInfo = { ...iphone, udid: "second-iphone", name: "iPhone 8 (2)" };

    const failures = await startFleetPreview([iphone, second]);

    expect(api.prepareDevice).toHaveBeenCalledTimes(2);
    expect(api.prepareDevice).toHaveBeenCalledWith("second-iphone");
    expect(failures.map((failure) => failure.udid)).toEqual([iphone.udid]);
  });

  it("reports nothing when the whole fleet starts", async () => {
    expect(await startFleetPreview([android, iphone])).toEqual([]);
  });
});
