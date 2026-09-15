import { beforeEach, describe, expect, it, vi } from "vitest";
import { waitFor } from "@testing-library/react";
import * as api from "./api";
import { pushToast, toastError } from "./toastStore";

import { buildDeviceActions, readAndAssignTikTokAccounts, type DeviceActionDeps } from "./deviceActions";
import { gateDeviceMenu, isSubmenu, menuLeaves, type DeviceMenuNode } from "./deviceMenu";
import type { DeviceInfo } from "./types";

vi.mock("./api", async importOriginal => ({...await importOriginal<typeof api>(), interactionReadAccount:vi.fn(),saveDeviceHandle:vi.fn(),listDeviceMetas:vi.fn()}));
vi.mock("./toastStore",()=>({pushToast:vi.fn(),toastError:vi.fn()}));

beforeEach(()=>{
  vi.clearAllMocks();
  vi.mocked(api.interactionReadAccount).mockImplementation(async udid=>({udid,expectedHandle:"old",observedHandle:`nick_${udid}`,status:"mismatch",checkedAt:"2026-09-16T00:00:00Z",snapshotSha256:"proof"}));
  vi.mocked(api.saveDeviceHandle).mockImplementation(async (_id,_expected,handle)=>handle);
  vi.mocked(api.listDeviceMetas).mockResolvedValue([]);
});

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
    setSyslogFor: vi.fn(),
    setHealthFor: vi.fn(),
    ...over,
  };
}

describe("buildDeviceActions", () => {
  it("reads the selected group when invoked on its tile, and only the clicked phone outside it",async()=>{
    const a=device({udid:"a"}), b=device({udid:"b"}), c=device({udid:"c"});
    const d=deps({selectedDevices:[a,b]});
    buildDeviceActions(a,d).find(n=>n.id==="read-tiktok-account")!.run!();
    await waitFor(()=>expect(api.saveDeviceHandle).toHaveBeenCalledTimes(2));
    expect(api.interactionReadAccount).toHaveBeenNthCalledWith(1,"a");
    expect(api.interactionReadAccount).toHaveBeenNthCalledWith(2,"b");
    await waitFor(()=>expect(pushToast).toHaveBeenCalledWith("ok","Đã gán nick TikTok 2/2 máy"));
    buildDeviceActions(c,d).find(n=>n.id==="read-tiktok-account")!.run!();
    await waitFor(()=>expect(api.saveDeviceHandle).toHaveBeenCalledWith("c","old","nick_c"));
  });

  it("reserves queued phones across repeated clicks and keeps the captured selection",async()=>{
    const a=device({udid:"a"}),b=device({udid:"b"});
    let finish!: (value:api.AccountReading)=>void;
    vi.mocked(api.interactionReadAccount).mockImplementationOnce(()=>new Promise(resolve=>{finish=resolve;}));
    const d=deps({selectedDevices:[a,b]});
    const first=readAndAssignTikTokAccounts([...d.selectedDevices!],d);
    await readAndAssignTikTokAccounts([a,b],d);
    d.selectedDevices=[device({udid:"c"})];
    expect(api.interactionReadAccount).toHaveBeenCalledTimes(1);
    finish({udid:"a",expectedHandle:"old",observedHandle:"nick_a",status:"mismatch",checkedAt:"",snapshotSha256:"proof"});
    await first;
    expect(api.interactionReadAccount).toHaveBeenCalledTimes(2);
    expect(api.saveDeviceHandle).toHaveBeenNthCalledWith(2,"b","old","nick_b");
  });

  it("keeps failed or unknown accounts and continues after stale saves and disconnects",async()=>{
    const targets=["a","b","c","d"].map(udid=>device({udid}));
    vi.mocked(api.interactionReadAccount).mockRejectedValueOnce(new Error("disconnected"))
      .mockResolvedValueOnce({udid:"b",expectedHandle:"old",observedHandle:null,status:"unknown",checkedAt:"",snapshotSha256:"proof"});
    vi.mocked(api.saveDeviceHandle).mockRejectedValueOnce(new Error("stale account"));
    await readAndAssignTikTokAccounts(targets,deps());
    expect(api.saveDeviceHandle).toHaveBeenCalledTimes(2);
    expect(api.saveDeviceHandle).toHaveBeenLastCalledWith("d","old","nick_d");
    expect(toastError).toHaveBeenCalledTimes(3);
    expect(pushToast).toHaveBeenLastCalledWith("warn","Đã gán nick TikTok 1/4 máy");
  });
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
