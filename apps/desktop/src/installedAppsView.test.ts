import { describe, expect, it } from "vitest";
import { installedAppsView } from "./installedAppsView";
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
