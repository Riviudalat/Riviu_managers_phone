import { beforeEach, describe, expect, it, vi } from "vitest";
import * as api from "./api";
import { requestConfirm, requestPrompt } from "./confirmStore";
import { pushToast, toastError } from "./toastStore";

import { buildDeviceActions, readAndAssignTikTokAccounts, type DeviceActionDeps } from "./deviceActions";
import { gateDeviceMenu, isSubmenu, menuLeaves, type DeviceMenuNode } from "./deviceMenu";
import type { DeviceInfo } from "./types";

vi.mock("./api", async importOriginal => ({ ...await importOriginal<typeof api>(), previewAccountReconciliation: vi.fn(), applyAccountReconciliation: vi.fn(), listDeviceMetas: vi.fn(), patchDeviceMeta: vi.fn() }));
vi.mock("./confirmStore", () => ({ requestConfirm: vi.fn(), requestPrompt: vi.fn() }));
vi.mock("./toastStore", () => ({ pushToast: vi.fn(), toastError: vi.fn() }));

beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(requestConfirm).mockResolvedValue(true);
  vi.mocked(api.previewAccountReconciliation).mockImplementation(async udids => ({ operationId: "op-proof", blocker: null,
    rows: udids.map(udid => ({ udid, expectedHandle: "old", observedHandle: `nick_${udid}`, checkedAt: "2026-10-06T00:00:00Z", snapshotSha256: "proof", error: null })) }));
  vi.mocked(api.applyAccountReconciliation).mockResolvedValue([{ udid: "a", expectedHandle: "old", observedHandle: "nick_a" }]);
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
    refreshMetas: vi.fn(async () => api.listDeviceMetas()),
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
  it("waits for the complete read batch without ordinary confirmation before applying its operation", async () => {
    let finish!: (value: api.AccountReconciliationPlan) => void;
    vi.mocked(api.previewAccountReconciliation).mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
    const running = readAndAssignTikTokAccounts([device({ udid: "a" }), device({ udid: "b" })], deps());
    await readAndAssignTikTokAccounts([device({ udid: "b" })], deps());
    expect(api.previewAccountReconciliation).toHaveBeenCalledTimes(1);
    expect(api.applyAccountReconciliation).not.toHaveBeenCalled();
    finish({ operationId: "batch", blocker: null, rows: [{ udid: "a", expectedHandle: "old", observedHandle: "next", checkedAt: "now", snapshotSha256: "proof", error: null }] });
    await running;
    expect(requestConfirm).not.toHaveBeenCalled();
    expect(api.applyAccountReconciliation).toHaveBeenCalledWith("batch");
  });

  it("reads conflicting exact serials only after consent and applies the fresh expanded plan", async () => {
    vi.mocked(api.previewAccountReconciliation).mockResolvedValueOnce({ operationId: "blocked", rows: [], blocker: {
      accountConflict: { conflictingDevices: [{ udid: "ce0517151215a00304" }] } } });
    await readAndAssignTikTokAccounts([device({ udid: "ce031713dd735a1103" })], deps());
    expect(api.previewAccountReconciliation).toHaveBeenNthCalledWith(2, ["ce031713dd735a1103", "ce0517151215a00304"]);
    expect(requestConfirm).toHaveBeenCalledTimes(1);
    expect(api.applyAccountReconciliation).toHaveBeenCalledWith("op-proof");
  });

  it("summarizes confirmed changes and unchanged mappings once with frozen failed numbers", async () => {
    const numbers = new Map([["a", 8], ["b", 13], ["c", 20]]);
    vi.mocked(api.previewAccountReconciliation).mockImplementationOnce(async () => {
      numbers.set("a", 99);
      return { operationId: "partial", blocker: null, rows: [
        { udid: "a", expectedHandle: "", observedHandle: null, checkedAt: null, snapshotSha256: null, error: { message: "offline" } },
        { udid: "b", expectedHandle: "old", observedHandle: "next", checkedAt: "now", snapshotSha256: "proof", error: null },
        { udid: "c", expectedHandle: "same", observedHandle: "same", checkedAt: "now", snapshotSha256: "proof", error: null },
      ] };
    });
    vi.mocked(api.applyAccountReconciliation).mockResolvedValue([
      { udid: "b", expectedHandle: "old", observedHandle: "next" },
      { udid: "c", expectedHandle: "same", observedHandle: "same" },
    ]);
    vi.mocked(api.listDeviceMetas).mockRejectedValue(new Error("refresh failed"));
    await readAndAssignTikTokAccounts([device({ udid: "a" }), device({ udid: "b" }), device({ udid: "c" })], deps({ deviceNumbers: numbers }));
    expect(requestConfirm).not.toHaveBeenCalled();
    expect(api.applyAccountReconciliation).toHaveBeenCalledExactlyOnceWith("partial");
    expect(toastError).not.toHaveBeenCalled();
    expect(pushToast).toHaveBeenCalledTimes(1);
    expect(pushToast).toHaveBeenCalledWith("warn", "Đã cập nhật thành công 2/3", expect.stringContaining("1 máy thay đổi · 1 máy đã đúng · 1 máy chưa cập nhật"), "Máy chưa cập nhật: 8");
    expect(vi.mocked(pushToast).mock.calls[0][2]).toContain("refresh failed");
  });

  it("does not expand or apply when conflict consent is declined", async () => {
    vi.mocked(requestConfirm).mockResolvedValue(false);
    vi.mocked(api.previewAccountReconciliation).mockResolvedValueOnce({ operationId: "blocked", rows: [], blocker: {
      accountConflict: { conflictingDevices: [{ udid: "outside" }] } } });
    await readAndAssignTikTokAccounts([device({ udid: "a" })], deps());
    expect(api.previewAccountReconciliation).toHaveBeenCalledTimes(1);
    expect(api.applyAccountReconciliation).not.toHaveBeenCalled();
  });

  it("retains operation identity without replay when apply acknowledgment is lost", async () => {
    vi.mocked(api.applyAccountReconciliation).mockRejectedValue(new Error("lost ACK"));
    await readAndAssignTikTokAccounts([device({ udid: "a" })], deps());
    expect(api.applyAccountReconciliation).toHaveBeenCalledTimes(1);
    expect(api.previewAccountReconciliation).toHaveBeenCalledTimes(1);
    expect(toastError).toHaveBeenCalledWith(expect.stringContaining("op-proof"), expect.any(Error));
  });
  it("uses serial identity for an unassigned number instead of reporting machine zero", async () => {
    vi.mocked(api.applyAccountReconciliation).mockResolvedValue([]);
    await readAndAssignTikTokAccounts([device({ udid: "pending-phone" })], deps({ deviceNumbers: new Map([["pending-phone", 0]]) }));
    expect(pushToast).toHaveBeenCalledWith("warn", "Đã cập nhật thành công 0/1", expect.stringContaining("pending-phone:"), "Máy chưa cập nhật: -phone");
  });
  it("reports a wholly failed read once without attempting an empty apply", async () => {
    vi.mocked(api.previewAccountReconciliation).mockResolvedValueOnce({ operationId: "empty", blocker: null, rows: [
      { udid: "a", expectedHandle: "old", observedHandle: null, checkedAt: null, snapshotSha256: null, error: { message: "offline" } },
    ] });
    await readAndAssignTikTokAccounts([device({ udid: "a" })], deps({ deviceNumbers: new Map([["a", 8]]) }));
    expect(api.applyAccountReconciliation).not.toHaveBeenCalled();
    expect(pushToast).toHaveBeenCalledExactlyOnceWith("warn", "Đã cập nhật thành công 0/1", expect.stringContaining("Máy 8 (a): offline"), "Máy chưa cập nhật: 8");
  });
  it("keeps current metadata after a successful rename whose refresh fails", async () => {
    const state = deps();
    vi.mocked(requestPrompt).mockResolvedValue("New name");
    vi.mocked(api.patchDeviceMeta).mockResolvedValue({ udid: "a", alias: "New name", notes: "", tags: [] });
    vi.mocked(api.listDeviceMetas).mockRejectedValue(new Error("read unavailable"));
    const action = menuLeaves(buildDeviceActions(device({udid: "a"}), state)).find(row => row.id === "rename")!;
    action.run!();
    await vi.waitFor(() => expect(api.patchDeviceMeta).toHaveBeenCalled());
    expect(state.refreshMetas).toHaveBeenCalledTimes(1);
    expect(pushToast).toHaveBeenCalledWith("ok", "Đã đổi tên thành “New name”");
    expect(toastError).toHaveBeenCalledWith(expect.stringContaining("làm mới"), expect.any(Error));
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
