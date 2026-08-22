import { describe, it, expect } from "vitest";
import { assign, leftover, splitText } from "./textDistribution";

describe("splitText", () => {
  it("splits on lines and drops blank lines", () => {
    expect(splitText("a\nb\r\n\n  \nc", { kind: "lines" })).toEqual(["a", "b", "c"]);
  });

  it("splits on a separator", () => {
    expect(splitText("a,b,,c", { kind: "separator", separator: "," })).toEqual(["a", "b", "c"]);
  });

  it("treats an empty separator as the whole block", () => {
    expect(splitText("hello", { kind: "separator", separator: "" })).toEqual(["hello"]);
  });

  it("splits on a regex", () => {
    expect(splitText("a1b22c", { kind: "regex", pattern: "\\d+" })).toEqual(["a", "b", "c"]);
  });

  it("throws on an invalid regex (caller surfaces it)", () => {
    expect(() => splitText("x", { kind: "regex", pattern: "(" })).toThrow();
  });

  it("preserves diacritics and inner spacing", () => {
    expect(splitText("Xin chào\nBạn khỏe không", { kind: "lines" })).toEqual([
      "Xin chào",
      "Bạn khỏe không",
    ]);
  });
});

describe("assign", () => {
  it("pairs by index up to the shorter list", () => {
    expect(assign(["m1", "m2", "m3"], ["u1", "u2"])).toEqual([
      { udid: "u1", text: "m1" },
      { udid: "u2", text: "m2" },
    ]);
  });

  it("pairs all when counts match", () => {
    expect(assign(["m1", "m2"], ["u1", "u2"])).toEqual([
      { udid: "u1", text: "m1" },
      { udid: "u2", text: "m2" },
    ]);
  });

  it("is empty when either side is empty", () => {
    expect(assign([], ["u1"])).toEqual([]);
    expect(assign(["m1"], [])).toEqual([]);
  });
});

describe("leftover", () => {
  it("counts unpaired items and devices", () => {
    expect(leftover(["m1", "m2", "m3"], ["u1"])).toEqual({ extraItems: 2, extraDevices: 0 });
    expect(leftover(["m1"], ["u1", "u2", "u3"])).toEqual({ extraItems: 0, extraDevices: 2 });
    expect(leftover(["m1", "m2"], ["u1", "u2"])).toEqual({ extraItems: 0, extraDevices: 0 });
  });
});
