import { describe, expect, it } from "vitest";
import {
  metaByUdid,
  orderDevicesByNumber,
  parseDeviceNumber,
  tileName,
  tileNumber,
} from "./deviceNaming";
import type { DeviceInfo, DeviceMeta } from "./types";

function device(udid: string, name = `phone ${udid}`): DeviceInfo {
  return {
    udid,
    name,
    model: "SM-G955F",
    platform: "android",
    osVersion: "9",
    connection: "usb",
    status: "ready",
    wdaReady: false,
  };
}

function meta(udid: string, over: Partial<DeviceMeta> = {}): DeviceMeta {
  return { udid, notes: "", tags: [], ...over };
}

describe("tileName", () => {
  it("prefers the operator's alias and falls back to what the phone reports", () => {
    expect(tileName(device("a", "SM-G955F"), meta("a", { alias: "Kệ trên · 3" }))).toBe(
      "Kệ trên · 3",
    );
    expect(tileName(device("a", "SM-G955F"), meta("a", { alias: "" }))).toBe("SM-G955F");
    expect(tileName(device("a", "SM-G955F"), undefined)).toBe("SM-G955F");
  });

  it("treats an alias of only spaces as no alias", () => {
    expect(tileName(device("a", "SM-G955F"), meta("a", { alias: "   " }))).toBe("SM-G955F");
  });
});

describe("tileNumber", () => {
  it("uses the operator's number, or the tile's position when unnumbered", () => {
    expect(tileNumber(7, meta("a", { number: 21 }))).toBe(21);
    expect(tileNumber(7, meta("a"))).toBe(7);
    expect(tileNumber(7, undefined)).toBe(7);
  });

  it("does not mistake a null number for a missing record", () => {
    expect(tileNumber(4, meta("a", { number: null }))).toBe(4);
  });
});

describe("orderDevicesByNumber", () => {
  it("leaves an unnumbered fleet exactly as it arrived", () => {
    const fleet = [device("a"), device("b"), device("c")];
    expect(orderDevicesByNumber(fleet, metaByUdid([])).map((d) => d.udid)).toEqual([
      "a",
      "b",
      "c",
    ]);
  });

  it("puts numbered phones first, in number order, and keeps the rest behind them", () => {
    const fleet = [device("a"), device("b"), device("c"), device("d")];
    const metas = metaByUdid([meta("c", { number: 1 }), meta("a", { number: 2 })]);
    expect(orderDevicesByNumber(fleet, metas).map((d) => d.udid)).toEqual(["c", "a", "b", "d"]);
  });

  /**
   * Nothing in the UI stops an operator numbering two phones 3. The grid must not scramble
   * when they do — the pair keeps the order the fleet list gave them.
   */
  it("keeps arrival order between phones that share a number", () => {
    const fleet = [device("a"), device("b"), device("c")];
    const metas = metaByUdid([
      meta("b", { number: 3 }),
      meta("a", { number: 3 }),
      meta("c", { number: 1 }),
    ]);
    expect(orderDevicesByNumber(fleet, metas).map((d) => d.udid)).toEqual(["c", "a", "b"]);
  });

  it("never loses or duplicates a tile", () => {
    const fleet = [device("a"), device("b"), device("c"), device("d"), device("e")];
    const metas = metaByUdid([meta("e", { number: 9 }), meta("b", { number: 1 })]);
    const ordered = orderDevicesByNumber(fleet, metas);
    expect(ordered).toHaveLength(5);
    expect(new Set(ordered.map((d) => d.udid)).size).toBe(5);
  });

  it("does not reorder the array it was given", () => {
    const fleet = [device("a"), device("b")];
    orderDevicesByNumber(fleet, metaByUdid([meta("b", { number: 1 })]));
    expect(fleet.map((d) => d.udid)).toEqual(["a", "b"]);
  });
});

describe("parseDeviceNumber", () => {
  it("reads a positive whole number", () => {
    expect(parseDeviceNumber("21")).toEqual({ number: 21 });
    expect(parseDeviceNumber("  7 ")).toEqual({ number: 7 });
  });

  it("treats an emptied field as clearing the number, not as an error", () => {
    expect(parseDeviceNumber("")).toEqual({ number: null });
    expect(parseDeviceNumber("   ")).toEqual({ number: null });
  });

  it("refuses what cannot be a phone's number, and says why", () => {
    for (const raw of ["0", "-3", "2.5", "12a", "abc"]) {
      const result = parseDeviceNumber(raw);
      expect("error" in result, `${raw} must be refused`).toBe(true);
    }
    expect(parseDeviceNumber("10000")).toEqual({ error: "Số máy tối đa là 9999." });
  });
});
