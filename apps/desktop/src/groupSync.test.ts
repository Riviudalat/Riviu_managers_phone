import { describe, it, expect } from "vitest";
import { defaultGroupSync, isNoopGroupSync, normalizeGroupSync } from "./groupSync";

describe("groupSync policy", () => {
  it("default is a noop", () => {
    expect(defaultGroupSync()).toEqual({ delay: { mode: "none" }, offset: { maxPx: 0 } });
    expect(isNoopGroupSync(defaultGroupSync())).toBe(true);
  });

  it("coerces garbage / unknown modes to a noop", () => {
    expect(normalizeGroupSync(null)).toEqual({
      delay: { mode: "none" },
      offset: { maxPx: 0 },
    });
    expect(normalizeGroupSync({ delay: { mode: "wat" }, offset: { maxPx: -5 } })).toEqual({
      delay: { mode: "none" },
      offset: { maxPx: 0 },
    });
  });

  it("keeps a valid random policy and rounds/clamps its fields", () => {
    expect(
      normalizeGroupSync({
        delay: { mode: "random", minMs: 10, maxMs: 20 },
        offset: { maxPx: 3 },
      }),
    ).toEqual({ delay: { mode: "random", minMs: 10, maxMs: 20 }, offset: { maxPx: 3 } });
    // Negative floored to 0, fractional rounded.
    expect(normalizeGroupSync({ delay: { mode: "random", minMs: -1, maxMs: 2.7 } }).delay).toEqual(
      { mode: "random", minMs: 0, maxMs: 3 },
    );
  });

  it("keeps a valid staggered policy", () => {
    expect(normalizeGroupSync({ delay: { mode: "staggered", stepMs: 150 } }).delay).toEqual({
      mode: "staggered",
      stepMs: 150,
    });
  });

  it("is not a noop when either delay or offset is active", () => {
    expect(
      isNoopGroupSync({ delay: { mode: "staggered", stepMs: 100 }, offset: { maxPx: 0 } }),
    ).toBe(false);
    expect(isNoopGroupSync({ delay: { mode: "none" }, offset: { maxPx: 4 } })).toBe(false);
  });
});
