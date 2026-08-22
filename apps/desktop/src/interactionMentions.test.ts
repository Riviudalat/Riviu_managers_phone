import { describe, expect, it } from "vitest";

import {
  parseMentions,
  resolveMentionActors,
  unionActors,
  type DeviceHandle,
} from "./interactionMentions";

describe("parseMentions", () => {
  it("strips @, splits on space/comma/semicolon, and dedups case-insensitively", () => {
    expect(parseMentions("@ann, bob;  @Ann   charlie")).toEqual(["ann", "bob", "charlie"]);
  });

  it("is empty for blank or separator-only input", () => {
    expect(parseMentions("")).toEqual([]);
    expect(parseMentions("  , ; @@ ")).toEqual([]);
  });

  it("keeps first-seen casing for display", () => {
    expect(parseMentions("@Bob @bob")).toEqual(["Bob"]);
  });
});

describe("resolveMentionActors", () => {
  const devices: DeviceHandle[] = [
    { udid: "u1", handle: "ann" },
    { udid: "u2", handle: "Bob" },
    { udid: "u3", handle: "" },
    { udid: "u4", handle: "carol" },
  ];

  it("matches mentions to owning phones, case-insensitively", () => {
    expect(resolveMentionActors(["ANN", "bob"], devices)).toEqual(["u1", "u2"]);
  });

  it("ignores mentions with no owning phone (external people)", () => {
    expect(resolveMentionActors(["stranger", "carol"], devices)).toEqual(["u4"]);
  });

  it("never matches a blank-handle device", () => {
    expect(resolveMentionActors([""], devices)).toEqual([]);
    expect(resolveMentionActors(["ann"], [{ udid: "x", handle: "  " }])).toEqual([]);
  });
});

describe("unionActors", () => {
  it("adds new udids without duplicating, preserving order", () => {
    expect(unionActors(["a", "b"], ["b", "c"])).toEqual(["a", "b", "c"]);
  });

  it("returns the base unchanged when there is nothing new", () => {
    expect(unionActors(["a", "b"], ["a"])).toEqual(["a", "b"]);
  });
});
