import { beforeEach, expect, it, vi } from "vitest";
const api = vi.hoisted(() => ({ listDeviceWorkStates: vi.fn(), listDeviceMetas: vi.fn(), setWallpaperBytes: vi.fn(), setWallpaper: vi.fn() }));
vi.mock("./api", () => api);
vi.mock("./numberWallpaper", () => ({ numberWallpaperPng: vi.fn(async () => new Uint8Array([1, 2, 3])) }));
import type { DeviceInfo } from "./types";
const devices = [{ udid: "a", platform: "android", status: "ready" }] as DeviceInfo[];
beforeEach(() => {
  vi.resetModules(); vi.clearAllMocks(); localStorage.clear();
  api.listDeviceMetas.mockResolvedValue([{ udid: "a", number: 21 }]);
  api.listDeviceWorkStates.mockResolvedValue([{udid: "a", currentOwner: null}]);
  api.setWallpaperBytes.mockResolvedValue(undefined);
});
it("waits for an observed idle device and does not reapply an acknowledged signature", async () => {
  const { setWallpaperSync, syncNumberWallpapers } = await import("./wallpaperSync");
  setWallpaperSync(["a"], true);
  api.listDeviceWorkStates.mockResolvedValueOnce([{udid: "a", currentOwner: "script"}]);
  await syncNumberWallpapers(devices);
  expect(api.setWallpaperBytes).not.toHaveBeenCalled();
  await syncNumberWallpapers(devices);
  await syncNumberWallpapers(devices);
  expect(api.setWallpaperBytes).toHaveBeenCalledTimes(1);
  expect(api.setWallpaperBytes).toHaveBeenCalledWith("a", [1, 2, 3]);
  expect(JSON.parse(localStorage.getItem("riviu.numberWallpaper.v1")!).a).toMatchObject({state: "applied", signature: "r-white-1:21"});
});
it("persists uncertainty before dispatch and requires explicit retry even after restart", async () => {
  let sync = await import("./wallpaperSync");
  sync.setWallpaperSync(["a"], true);
  api.setWallpaperBytes.mockImplementationOnce(async () => {
    expect(JSON.parse(localStorage.getItem("riviu.numberWallpaper.v1")!).a.state).toBe("pending");
    throw new Error("lost ACK");
  });
  await sync.syncNumberWallpapers(devices);
  vi.resetModules();
  sync = await import("./wallpaperSync");
  await sync.syncNumberWallpapers(devices);
  expect(api.setWallpaperBytes).toHaveBeenCalledTimes(1);
  sync.retryWallpaperSync(["a"]);
  await sync.syncNumberWallpapers(devices);
  expect(api.setWallpaperBytes).toHaveBeenCalledTimes(2);
});

it("does not retry healthy siblings or let manual input bypass uncertain ownership", async () => {
  const sync = await import("./wallpaperSync");
  sync.setWallpaperSync(["a", "b"], true);
  await sync.applyNumberWallpaper("b", 22);
  api.setWallpaperBytes.mockRejectedValueOnce(new Error("lost ACK"));
  await expect(sync.applyNumberWallpaper("a", 21)).rejects.toThrow("lost ACK");
  await expect(sync.applyCustomWallpaper("a", "photo.png")).rejects.toThrow();
  expect(api.setWallpaper).not.toHaveBeenCalled();
  sync.retryWallpaperSync(["a", "b"]);
  expect(JSON.parse(localStorage.getItem("riviu.numberWallpaper.v1")!).b).toMatchObject({state: "applied", signature: "r-white-1:22"});
  await sync.applyCustomWallpaper("b", "photo.png");
  expect(JSON.parse(localStorage.getItem("riviu.numberWallpaper.v1")!).b).toMatchObject({enabled: false, signature: "custom", state: "applied"});
});

it("explicit manual apply can restore the same number while automatic observation stays deduplicated", async () => {
  const sync = await import("./wallpaperSync");
  sync.setWallpaperSync(["a"], true);
  await sync.applyNumberWallpaper("a", 21);
  await sync.applyNumberWallpaper("a", 21);
  expect(api.setWallpaperBytes).toHaveBeenCalledTimes(2);
  await sync.syncNumberWallpapers(devices);
  expect(api.setWallpaperBytes).toHaveBeenCalledTimes(2);
});
