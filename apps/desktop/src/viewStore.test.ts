import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  collectDepartedViews,
  collectPaintReports,
  collectStalledViews,
  nextViewReconnectDelay,
  PAINT_STALL_MS,
  startViewClient,
  VIEW_RECONNECT_MAX_MS,
  VIEW_RECONNECT_MIN_MS,
} from "./viewStore";

// A whole-module factory: vitest throws on any export this omits, and `startViewClient`
// below reaches into it. Every api function `viewStore` imports must be listed here.
vi.mock("./api", () => ({
  viewEndpoint: vi.fn(async () => null),
  viewReportPaint: vi.fn(async () => undefined),
}));

describe("view WebSocket reconnect", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("backs off from 200 ms to about 2 s", () => {
    expect(nextViewReconnectDelay(VIEW_RECONNECT_MIN_MS)).toBe(400);
    expect(nextViewReconnectDelay(400)).toBe(800);
    expect(nextViewReconnectDelay(800)).toBe(1600);
    expect(nextViewReconnectDelay(1600)).toBe(VIEW_RECONNECT_MAX_MS);
    expect(nextViewReconnectDelay(VIEW_RECONNECT_MAX_MS)).toBe(VIEW_RECONNECT_MAX_MS);
  });

  it("tries the endpoint once in test mode and does not reconnect", async () => {
    const api = await import("./api");
    startViewClient();
    await vi.waitFor(() => expect(api.viewEndpoint).toHaveBeenCalledTimes(1));
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(api.viewEndpoint).toHaveBeenCalledTimes(1);
  });
});

describe("stalled view detection", () => {
  // The defect being guarded, measured on hardware: a producer restarted at a new
  // resolution, decoded exactly one keyframe, and then painted nothing for 8 minutes while
  // the Rust watchdog stayed silent -- it counts bytes arriving from the phone, not frames
  // drawn on screen, so a stream that arrives and cannot be decoded looks perfectly healthy
  // to it. The canvas held a stale frame the whole time, which reads as a live phone that
  // simply is not changing.
  const now = 1_000_000;

  const beat = (at: number, receivedCount: number, frames: number, generation = 1) => ({
    at,
    generation,
    received: receivedCount,
    frames,
  });

  it("flags a view whose packets kept arriving while it drew nothing", () => {
    const painted = new Map([["stale", beat(now - PAINT_STALL_MS - 1, 100, 50)]]);
    const latest = new Map([["stale", beat(now, 340, 50)]]);
    expect(collectStalledViews(now, ["stale"], painted, latest)).toEqual(["stale"]);
  });

  it("does not call one isolated codec packet a stalled video", () => {
    const painted = new Map([["config", beat(now - PAINT_STALL_MS - 1, 100, 50)]]);
    const latest = new Map([["config", beat(now, 101, 50)]]);
    expect(collectStalledViews(now, ["config"], painted, latest)).toEqual([]);
  });

  it("leaves a static screen alone even though it has drawn nothing for ages", () => {
    // The mistake the first version of this rule made, and the expensive one: scrcpy only
    // encodes when the screen changes, so a phone parked on a lock screen sends nothing and
    // paints nothing and is entirely healthy. Restarting it cost ~45s of real downtime.
    const painted = new Map([["idle", beat(now - 10 * PAINT_STALL_MS, 100, 50)]]);
    const latest = new Map([["idle", beat(now, 100, 50)]]);
    expect(collectStalledViews(now, ["idle"], painted, latest)).toEqual([]);
  });

  it("leaves a view alone while it is still painting", () => {
    const painted = new Map([["fresh", beat(now - 1000, 100, 50)]]);
    const latest = new Map([["fresh", beat(now, 124, 74)]]);
    expect(collectStalledViews(now, ["fresh"], painted, latest)).toEqual([]);
  });

  it("leaves a view exactly at the boundary alone", () => {
    const painted = new Map([["edge", beat(now - PAINT_STALL_MS, 100, 50)]]);
    const latest = new Map([["edge", beat(now, 300, 50)]]);
    expect(collectStalledViews(now, ["edge"], painted, latest)).toEqual([]);
  });

  it("does not flag a view that has never painted", () => {
    const latest = new Map([["starting", beat(now, 5, 0)]]);
    expect(collectStalledViews(now, ["starting"], new Map(), latest)).toEqual([]);
  });

  it("ignores a stalled view that is not live", () => {
    const painted = new Map([["gone", beat(now - 10 * PAINT_STALL_MS, 100, 50)]]);
    const latest = new Map([["gone", beat(now, 900, 50)]]);
    expect(collectStalledViews(now, [], painted, latest)).toEqual([]);
  });

  it("keeps the stall window well above a static screen's quiet period", () => {
    // scrcpy only encodes when the screen changes, so a phone on a static screen paints
    // nothing for a while quite legitimately. A window at or below the 5s byte threshold
    // would restart healthy streams.
    expect(PAINT_STALL_MS).toBeGreaterThan(5000);
  });
});

describe("paint reports sent to the host watchdog", () => {
  const now = 1_000_000;
  const beat = (at: number, receivedCount: number, frames: number, generation = 1) => ({
    at,
    generation,
    received: receivedCount,
    frames,
  });

  it("reports healthy devices too, so silence means nobody is watching", () => {
    // The distinction the host cannot make for itself: "no device is stalled" and "no window
    // is reporting" must lead to different behaviour, because the second one has to fall
    // back to the coarse byte rule rather than trust a paint rule nothing is feeding.
    const latest = new Map([
      ["healthy", beat(now, 300, 300)],
      ["stalled", beat(now, 900, 200)],
    ]);
    const painted = new Map([
      ["healthy", beat(now - 20, 300, 300)],
      ["stalled", beat(now - 30_000, 200, 200)],
    ]);
    const reports = collectPaintReports(now, latest, painted);
    expect(reports.map((report) => report.udid).sort()).toEqual(["healthy", "stalled"]);
  });

  it("sends the generation, so the host can date the evidence", () => {
    // Counters captured before a restart show arrivals far ahead of frames forever. Without
    // the generation the host would act on them the instant the restart completed, which is
    // the 291-restart loop with an extra hop in it.
    const latest = new Map([["a", beat(now, 900, 200, 7)]]);
    const painted = new Map([["a", beat(now - 30_000, 200, 200, 7)]]);
    expect(collectPaintReports(now, latest, painted)[0]).toMatchObject({
      generation: 7,
      received: 900,
      frames: 200,
      packetsSincePaint: 700,
      sincePaintMs: 30_000,
    });
  });

  it("sends an age rather than a timestamp", () => {
    // The WebView's clock and the host's are different clocks; comparing them across the IPC
    // boundary is how a sleeping laptop becomes a fleet-wide restart. Never negative, even
    // if the beat is somehow ahead of `now`.
    const latest = new Map([["a", beat(now + 5_000, 10, 10)]]);
    expect(collectPaintReports(now, latest, new Map())[0].sincePaintMs).toBe(0);
  });

  it("dates a device that has never painted from its own first beat", () => {
    // `frames === 0` is what tells the host this is a device starting up. The age must not
    // be an invented zero, or a device that never paints at all would look freshly drawn.
    const latest = new Map([["starting", beat(now - 4_000, 5, 0)]]);
    const report = collectPaintReports(now, latest, new Map())[0];
    expect(report.frames).toBe(0);
    expect(report.sincePaintMs).toBe(4_000);
  });
});

describe("forgetting devices that left the fleet", () => {
  const beat = (at: number) => ({ at, generation: 1, received: 10, frames: 10 });

  function memory() {
    return {
      sizes: new Map([
        ["stays", { width: 1080, height: 2400, generation: 1 }],
        ["left", { width: 1080, height: 2220, generation: 1 }],
      ]),
      live: new Set(["stays", "left"]),
      decodeFailed: new Set(["left"]),
      lastPaintBeat: new Map([["stays", beat(1)], ["left", beat(1)]]),
      latestBeat: new Map([["stays", beat(2)], ["left", beat(2)]]),
    };
  }

  it("drops every trace of a departed phone and keeps the rest untouched", () => {
    // Nothing here was ever pruned, and two of these stores are load-bearing. `live` says
    // whether the tile claims the stream is up, so a phone that went away while live came
    // back *already* live -- a white canvas labelled as working. `latestBeat` is what the
    // host's watchdog is handed every two seconds, so it kept receiving evidence about
    // devices that had left, for the life of the page.
    const stores = memory();

    expect(collectDepartedViews(new Set(["stays"]), stores)).toEqual(["left"]);

    expect(stores.live.has("left")).toBe(false);
    expect(stores.sizes.has("left")).toBe(false);
    expect(stores.decodeFailed.has("left")).toBe(false);
    expect(stores.lastPaintBeat.has("left")).toBe(false);
    expect(stores.latestBeat.has("left")).toBe(false);

    expect(stores.live.has("stays")).toBe(true);
    expect(stores.sizes.get("stays")?.height).toBe(2400);
    expect(stores.latestBeat.has("stays")).toBe(true);
  });

  it("stops the departed phone reaching the host's watchdog", () => {
    // The two rules together: what the watchdog is sent is exactly what `latestBeat` holds,
    // so pruning is what actually stops the reports rather than merely tidying up.
    const stores = memory();
    collectDepartedViews(new Set(["stays"]), stores);

    const reports = collectPaintReports(1_000, stores.latestBeat, stores.lastPaintBeat);

    expect(reports.map((report) => report.udid)).toEqual(["stays"]);
  });

  it("forgets nothing when every device is still present", () => {
    const stores = memory();
    expect(collectDepartedViews(new Set(["stays", "left"]), stores)).toEqual([]);
    expect(stores.live.size).toBe(2);
  });
});
