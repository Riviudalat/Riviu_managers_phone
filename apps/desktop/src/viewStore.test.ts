import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  nextViewReconnectDelay,
  startViewClient,
  VIEW_RECONNECT_MAX_MS,
  VIEW_RECONNECT_MIN_MS,
} from "./viewStore";

vi.mock("./api", () => ({
  viewEndpoint: vi.fn(async () => null),
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
