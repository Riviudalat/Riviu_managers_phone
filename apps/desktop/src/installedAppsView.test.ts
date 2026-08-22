import { describe, expect, it } from "vitest";
import { installedAppsFootnote, installedAppsView } from "./installedAppsView";
import type { InstalledApp } from "./types";

const REDMI: InstalledApp[] = [
  { bundleId: "com.ss.android.ugc.trill", kind: "user", label: null },
  { bundleId: "com.riviu.agent", kind: "user", label: null },
  { bundleId: "com.android.settings", kind: "system", label: null },
];

describe("installedAppsView", () => {
  it("shows installed apps and counts system ones without listing them", () => {
    const view = installedAppsView({ state: "ready", apps: REDMI }, false, "");

    expect(view.rows.map((app) => app.bundleId)).toEqual([
      "com.ss.android.ugc.trill",
      "com.riviu.agent",
    ]);
    expect(view.userCount).toBe(2);
    expect(view.systemCount).toBe(1);
    expect(view.notice).toBeNull();
  });

  it("keeps the order the phone reported rather than sorting", () => {
    // The listing order is the phone's own; re-sorting would invent an ordering and make
    // two reads of the same phone look different for no reason.
    const view = installedAppsView({ state: "ready", apps: REDMI }, true, "");

    expect(view.rows.map((app) => app.bundleId)).toEqual(REDMI.map((app) => app.bundleId));
  });

  it("tells a refusal apart from an empty phone", () => {
    // This is the distinction the backend refuses in order to preserve. Collapsing them
    // would render "this phone has nothing installed" for an iPhone nobody can enumerate.
    const refused = installedAppsView(
      {
        state: "failed",
        reason: "capability listInstalledApps is not supported by this driver",
      },
      false,
      "",
    );
    const empty = installedAppsView({ state: "ready", apps: [] }, false, "");

    expect(refused.notice?.kind).toBe("refused");
    expect(refused.notice?.text).toContain("listInstalledApps");
    expect(empty.notice?.kind).toBe("empty");
    expect(refused.notice?.text).not.toEqual(empty.notice?.text);
  });

  it("tells an empty phone apart from a filter that matches nothing", () => {
    const filtered = installedAppsView({ state: "ready", apps: REDMI }, false, "zzz");

    expect(filtered.notice?.kind).toBe("filtered");
    // The counts stay real, so the operator can see the filter is what emptied the list.
    expect(filtered.userCount).toBe(2);
  });

  it("filters on the bundle id, and on the label when a platform gives one", () => {
    const withLabel: InstalledApp[] = [
      { bundleId: "com.ss.iphone.ugc.Ame", kind: "user", label: "TikTok" },
    ];

    expect(installedAppsView({ state: "ready", apps: REDMI }, false, "TRILL").rows).toHaveLength(
      1,
    );
    expect(
      installedAppsView({ state: "ready", apps: withLabel }, false, "tiktok").rows,
    ).toHaveLength(1);
  });

  it("says nothing about counts while still loading", () => {
    const view = installedAppsView({ state: "loading" }, false, "");

    expect(view.notice?.kind).toBe("loading");
    expect(view.rows).toEqual([]);
    expect(view.userCount).toBe(0);
  });
});

/**
 * The footnote used to be one unconditional sentence saying Android gives no names, which was
 * true while the only route was adb. The helper reads them off `PackageManager` now, so the
 * wrong sentence is worse than none: telling an operator looking at "Zalo" that names are
 * unavailable reads as the panel not knowing what it is showing.
 */
describe("installedAppsFootnote", () => {
  it("says nothing when every row is named", () => {
    expect(
      installedAppsFootnote([
        { bundleId: "com.zing.zalo", kind: "user", label: "Zalo" },
        { bundleId: "com.ss.android.ugc.trill", kind: "user", label: "TikTok" },
      ]),
    ).toBeNull();
  });

  it("names the missing helper when nothing is named", () => {
    const text = installedAppsFootnote([
      { bundleId: "com.zing.zalo", kind: "user", label: null },
    ]);
    expect(text).toContain("Riviu helper");
  });

  /**
   * The driver deliberately does not pay to name the system partition (4,5 s for 539 packages
   * against 3,6 s for the 162 a farm operator launches). Saying "the phone did not give a
   * name" for those would blame the phone for a choice this app made — measured on a fleet
   * Galaxy where the panel read "241 app máy không trả tên" and every one of them was system.
   */
  it("says system names were not asked for, rather than blaming the phone", () => {
    const text = installedAppsFootnote([
      { bundleId: "a", kind: "user", label: "Zalo" },
      { bundleId: "b", kind: "system", label: null },
      { bundleId: "c", kind: "system", label: "   " },
    ]);
    expect(text).toBe(
      "2 app hệ thống chưa đọc tên (để không mất thêm ~4,5 s mỗi máy) — hiện bằng tên gói.",
    );
  });

  it("still counts plainly when an unnamed row is a user app", () => {
    const text = installedAppsFootnote([
      { bundleId: "a", kind: "user", label: "Zalo" },
      { bundleId: "b", kind: "user", label: null },
      { bundleId: "c", kind: "system", label: null },
    ]);
    expect(text).toBe("2 app máy không trả tên, hiện bằng tên gói.");
  });

  it("says nothing about an empty list", () => {
    expect(installedAppsFootnote([])).toBeNull();
  });
});
