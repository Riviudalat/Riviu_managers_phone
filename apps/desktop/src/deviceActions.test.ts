import { describe, expect, it, vi } from "vitest";

import { buildDeviceActions, type DeviceActionDeps } from "./deviceActions";
import { gateDeviceMenu, isSubmenu, menuLeaves, type DeviceMenuNode } from "./deviceMenu";
import type { DeviceInfo } from "./types";

/**
 * The catalog moved out of `App.tsx` so it could be reached without mounting the app.
 *
 * That is the whole point of the move, and these are the tests it makes possible: 696 lines
 * of menu rows previously had no test of their own, because reaching a single row meant
 * rendering the entire shell first.
 */

/** Every node in the tree, submenu rows included — `menuLeaves` deliberately skips those. */
function everyNode(nodes: DeviceMenuNode[]): DeviceMenuNode[] {
  const out: DeviceMenuNode[] = [];
  for (const node of nodes) {
    out.push(node);
    if (node.children?.length) out.push(...everyNode(node.children));
  }
  return out;
}

function device(over: Partial<DeviceInfo> = {}): DeviceInfo {
  return {
    udid: "98895a3355424e484f",
    name: "May 01",
    model: "SM-A032F",
    platform: "android",
    ...over,
  } as unknown as DeviceInfo;
}

function deps(over: Partial<DeviceActionDeps> = {}): DeviceActionDeps {
  return {
    reload: vi.fn(async () => undefined),
    metaMap: new Map(),
    metas: [],
    setMetas: vi.fn(),
    controlCenter: null,
    setControlCenter: vi.fn(),
    groupMode: false,
    setFocusUdid: vi.fn(),
    setFilesFor: vi.fn(),
    setAdbFor: vi.fn(),
    ...over,
  };
}

describe("buildDeviceActions", () => {
  it("still offers the whole catalog after the move", () => {
    // A guard on the four tests below: every one of them would pass vacuously against an
    // empty list, and an extraction that silently dropped rows is exactly the failure this
    // move could have caused.
    const nodes = everyNode(buildDeviceActions(device(), deps()));
    expect(nodes.length).toBeGreaterThan(30);
    expect(menuLeaves(buildDeviceActions(device(), deps())).length).toBeGreaterThan(25);
  });

  it("gives every row an id that is unique across the whole tree", () => {
    // Two rows sharing an id is not a cosmetic bug: search flattens submenus into one list,
    // so a duplicate makes one of the two unreachable.
    const ids = everyNode(buildDeviceActions(device(), deps())).map((n) => n.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("offers no row without something behind it", () => {
    // The rule `deviceMenu.ts` states: a row exists only if a command exists. A leaf with no
    // `run` is a label the operator can click for nothing.
    const dead = menuLeaves(buildDeviceActions(device(), deps())).filter(
      (n) => !n.run && !n.disabled && !isSubmenu(n),
    );
    expect(dead.map((n) => n.id)).toEqual([]);
  });

  it("drops the Android-only rows for an iPhone and keeps the rest", () => {
    const android = gateDeviceMenu(buildDeviceActions(device(), deps()), "android");
    const ios = gateDeviceMenu(
      buildDeviceActions(device({ platform: "ios" }), deps()),
      "ios",
    );
    expect(everyNode(ios).length).toBeGreaterThan(0);
    expect(everyNode(ios).length).toBeLessThan(everyNode(android).length);
    expect(everyNode(ios).some((n) => n.androidOnly)).toBe(false);
  });

  it("marks the rows that cannot be taken back as danger", () => {
    // Reboot and power off end every session running on that phone. The confirm dialog is
    // driven by `danger`, so an unmarked row is one that fires on a single click.
    const byId = new Map(everyNode(buildDeviceActions(device(), deps())).map((n) => [n.id, n]));
    for (const id of ["reboot", "power-off"]) {
      expect(byId.get(id), `row ${id} is missing`).toBeDefined();
      expect(byId.get(id)?.danger, `row ${id} is not marked danger`).toBe(true);
    }
  });

  it("reads the control-centre row's label from the state it was built with", () => {
    // The row reverses itself depending on `controlCenter`, which is why it was in the
    // callback's dependency list. Passing that state in explicitly is what lets the two
    // labels be compared at all.
    const label = (d: DeviceActionDeps) =>
      everyNode(buildDeviceActions(device(), d)).find((n) => n.id === "control-center")?.label;
    const off = label(deps({ controlCenter: null }));
    const on = label(deps({ controlCenter: "98895a3355424e484f" }));
    expect(off).toBeDefined();
    expect(on).toBeDefined();
    expect(on).not.toBe(off);
  });
});
