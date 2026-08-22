import { describe, expect, it } from "vitest";
import {
  filterDeviceMenu,
  foldAccents,
  gateDeviceMenu,
  isSubmenu,
  menuLeaves,
  menuNodeMatches,
  withoutMenuIds,
  type DeviceMenuNode,
} from "./deviceMenu";

/** A miniature of the real menu: two plain rows, one submenu, one iOS-hostile submenu. */
function sample(): DeviceMenuNode[] {
  return [
    { id: "open", label: "Mở điều khiển" },
    { id: "shot", label: "Chụp màn hình về máy tính", keywords: "screenshot" },
    {
      id: "adb",
      label: "ADB",
      androidOnly: true,
      children: [
        { id: "wifi-on", label: "Bật Wi-Fi trên máy", keywords: "wifi", androidOnly: true },
        { id: "dpi", label: "Đặt lại mật độ điểm", keywords: "dpi density", androidOnly: true },
      ],
    },
    {
      id: "more",
      label: "Chức năng khác",
      children: [
        { id: "settings", label: "Mở Cài đặt của máy", androidOnly: true },
        { id: "copy", label: "Sao chép ID máy" },
      ],
    },
  ];
}

describe("foldAccents", () => {
  it("folds Vietnamese so an unaccented keyboard can search", () => {
    expect(foldAccents("Cài đặt")).toBe("cai dat");
    expect(foldAccents("Chụp màn hình")).toBe("chup man hinh");
    // đ is its own codepoint, so NFD alone leaves it — the reason for the explicit pass.
    expect(foldAccents("Đổi bàn phím")).toBe("doi ban phim");
  });
});

describe("menuNodeMatches", () => {
  it("matches the label without accents and the keywords too", () => {
    const node: DeviceMenuNode = { id: "x", label: "Đặt lại mật độ điểm", keywords: "dpi density" };
    expect(menuNodeMatches(node, "mat do")).toBe(true);
    expect(menuNodeMatches(node, "DPI")).toBe(true);
    expect(menuNodeMatches(node, "resolution")).toBe(false);
  });

  it("an empty query matches everything, so a blank box is not a filter", () => {
    expect(menuNodeMatches({ id: "x", label: "bất kỳ" }, "   ")).toBe(true);
  });
});

describe("filterDeviceMenu", () => {
  it("returns the tree untouched when nothing is typed", () => {
    const menu = sample();
    expect(filterDeviceMenu(menu, "")).toBe(menu);
  });

  it("lifts a matching row out of its submenu and says where it came from", () => {
    const hits = filterDeviceMenu(sample(), "wifi");
    expect(hits.map((node) => node.id)).toEqual(["wifi-on"]);
    expect(hits[0].pathLabel).toBe("ADB › Bật Wi-Fi trên máy");
    expect(hits[0].children).toBeUndefined();
  });

  it("keeps a whole submenu when the submenu's own name is what matched", () => {
    const hits = filterDeviceMenu(sample(), "adb");
    expect(hits.map((node) => node.id)).toEqual(["adb"]);
    expect(hits[0].children).toHaveLength(2);
  });

  it("finds rows in different submenus at once", () => {
    const hits = filterDeviceMenu(sample(), "may");
    // "Chụp màn hình về máy tính", "Bật Wi-Fi trên máy", "Mở Cài đặt của máy", "Sao chép ID máy"
    expect(hits.map((node) => node.id)).toEqual(["shot", "wifi-on", "settings", "copy"]);
  });

  it("answers empty rather than everything when nothing matches", () => {
    expect(filterDeviceMenu(sample(), "zzzz")).toEqual([]);
  });
});

describe("gateDeviceMenu", () => {
  it("leaves an Android phone the whole menu", () => {
    expect(menuLeaves(gateDeviceMenu(sample(), "android")).map((node) => node.id)).toEqual([
      "open",
      "shot",
      "wifi-on",
      "dpi",
      "settings",
      "copy",
    ]);
  });

  /**
   * The half that is easy to forget: dropping the rows is not enough, because a submenu
   * whose every row went away would still render as a chevron that opens onto nothing.
   */
  it("drops a submenu that emptied out, not just its rows", () => {
    const gated = gateDeviceMenu(sample(), "ios");
    expect(gated.map((node) => node.id)).toEqual(["open", "shot", "more"]);
    const more = gated.find((node) => node.id === "more");
    expect(more?.children?.map((node) => node.id)).toEqual(["copy"]);
  });

  it("does not mutate the menu it was given", () => {
    const menu = sample();
    gateDeviceMenu(menu, "ios");
    expect(menu[2].children).toHaveLength(2);
  });
});

describe("withoutMenuIds", () => {
  it("drops a top-level row and a nested one alike", () => {
    const kept = withoutMenuIds(sample(), ["open", "dpi"]);
    expect(kept.map((node) => node.id)).toEqual(["shot", "adb", "more"]);
    expect(kept[1].children?.map((node) => node.id)).toEqual(["wifi-on"]);
  });

  /**
   * The reason this exists rather than a filter at the call site: the focus overlay drops the
   * adb console because it has its own, and a submenu left with nothing in it would render as
   * a chevron opening onto nothing.
   */
  it("drops a submenu whose every row was taken", () => {
    const kept = withoutMenuIds(sample(), ["wifi-on", "dpi"]);
    expect(kept.map((node) => node.id)).toEqual(["open", "shot", "more"]);
  });

  it("leaves the menu alone when nothing matches", () => {
    expect(withoutMenuIds(sample(), ["nope"]).map((node) => node.id)).toEqual([
      "open",
      "shot",
      "adb",
      "more",
    ]);
  });

  it("does not mutate the menu it was given", () => {
    const menu = sample();
    withoutMenuIds(menu, ["dpi"]);
    expect(menu[2].children).toHaveLength(2);
  });
});

describe("isSubmenu", () => {
  it("counts a row that must go and ask the phone", () => {
    expect(isSubmenu({ id: "a", label: "Ứng dụng", loadChildren: async () => [] })).toBe(true);
    expect(isSubmenu({ id: "a", label: "ADB", children: [{ id: "b", label: "x" }] })).toBe(true);
    expect(isSubmenu({ id: "a", label: "Chụp" })).toBe(false);
    // An explicitly empty list is not a submenu: it would open onto nothing.
    expect(isSubmenu({ id: "a", label: "x", children: [] })).toBe(false);
  });
});
