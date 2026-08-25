import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { GroupManagerPopup } from "./GroupManagerPopup";
import type { DeviceGroup, DeviceInfo, DeviceMeta } from "../types";

/**
 * The rules this screen exists to make true, asserted as rules.
 *
 * Groups decide two things an operator cannot see afterwards: which phones run together, and
 * — because the interaction panel loads a group straight into its actor list — the order they
 * reply to each other in. Both were previously decided by accident: there was no way to create
 * a group at all, and membership came back in SQLite's serial-number order.
 */

const saveGroup = vi.fn(async (group: DeviceGroup) => group);
const deleteGroup = vi.fn(async (_id: string) => undefined);
const confirm = vi.fn(async (_options: unknown) => true);

vi.mock("../api", () => ({
  saveGroup: (group: DeviceGroup) => saveGroup(group),
  deleteGroup: (id: string) => deleteGroup(id),
}));
vi.mock("../confirmStore", () => ({
  requestConfirm: (options: unknown) => confirm(options as never),
}));
vi.mock("../toastStore", () => ({
  pushToast: vi.fn(),
  toastError: vi.fn(),
}));

function device(udid: string, name: string): DeviceInfo {
  return {
    udid,
    name,
    model: "SM-G950F",
    platform: "android",
    osVersion: "9",
    connection: "usb",
    status: "ready",
    wdaReady: false,
  } as unknown as DeviceInfo;
}

/// Deliberately listed out of number order — fleet order is USB enumeration order.
const devices = [device("u-e", "E"), device("u-a", "A"), device("u-d", "D")];
const metas = new Map<string, DeviceMeta>([
  ["u-a", { udid: "u-a", notes: "", tags: [], number: 1 }],
  ["u-d", { udid: "u-d", notes: "", tags: [], number: 4 }],
  ["u-e", { udid: "u-e", notes: "", tags: [], number: 2 }],
]);

function group(id: string, name: string, udids: string[]): DeviceGroup {
  return { id, name, color: "#f97316", udids, createdAt: "2026-08-25T00:00:00Z" };
}

afterEach(cleanup);
beforeEach(() => {
  saveGroup.mockClear();
  deleteGroup.mockClear();
  confirm.mockClear();
  confirm.mockResolvedValue(true);
});

function open(groups: DeviceGroup[]) {
  const onChanged = vi.fn();
  render(
    <GroupManagerPopup
      devices={devices}
      groups={groups}
      metas={metas}
      onChanged={onChanged}
      onClose={() => undefined}
    />,
  );
  return onChanged;
}

describe("GroupManagerPopup", () => {
  /// Phones are listed by the number written on them, not by the order USB reported them.
  it("lists phones in number order, not fleet order", () => {
    open([group("g1", "Nhóm 1", [])]);
    const numbers = screen
      .getAllByRole("button")
      .map((element) => element.querySelector(".group-chip-num")?.textContent)
      .filter((value): value is string => Boolean(value));
    expect(numbers).toEqual(["1", "2", "4"]);
  });

  /// **The stored order is number order, whatever order the operator clicked in.**
  ///
  /// `partition_actors` slices the actor list in order and a `Chain` replies down it, so this
  /// is the difference between "nhóm A runs 1 → 2 → 4" and "nhóm A runs in serial order".
  it("stores a group in number order even when picked out of order", async () => {
    open([group("g1", "Nhóm 1", ["u-d"])]);
    // g1 already holds máy 4; add máy 1, which sorts before it.
    fireEvent.click(screen.getByTitle("A"));
    await waitFor(() => expect(saveGroup).toHaveBeenCalledTimes(1));
    expect(saveGroup.mock.calls[0][0].udids).toEqual(["u-a", "u-d"]);
  });

  /// A phone already in another group is shown as taken, and moving it asks first.
  it("asks before taking a phone out of the group it is in", async () => {
    open([group("g1", "Nhóm 1", []), group("g2", "Nhóm 2", ["u-a"])]);
    // `Nhóm 1` is the first group, so it is already the open one — clicking its header
    // here would close it and take the phone chips off screen.
    fireEvent.click(screen.getByTitle("A — đang ở nhóm Nhóm 2"));

    await waitFor(() => expect(confirm).toHaveBeenCalledTimes(1));
    expect(confirm.mock.calls[0][0]).toMatchObject({
      message: expect.stringContaining("Nhóm 2"),
    });
    await waitFor(() => expect(saveGroup).toHaveBeenCalledTimes(1));
  });

  /// Declining the move writes nothing — the confirm is a decision, not a formality.
  it("writes nothing when the move is declined", async () => {
    confirm.mockResolvedValue(false);
    open([group("g1", "Nhóm 1", []), group("g2", "Nhóm 2", ["u-a"])]);
    // `Nhóm 1` is the first group, so it is already the open one — clicking its header
    // here would close it and take the phone chips off screen.
    fireEvent.click(screen.getByTitle("A — đang ở nhóm Nhóm 2"));

    await waitFor(() => expect(confirm).toHaveBeenCalledTimes(1));
    expect(saveGroup).not.toHaveBeenCalled();
  });

  /// Taking a phone out of the open group is not a move, so it must not ask.
  it("does not ask when removing a phone from the group it is in", async () => {
    open([group("g1", "Nhóm 1", ["u-a"])]);
    fireEvent.click(screen.getByTitle("A"));

    await waitFor(() => expect(saveGroup).toHaveBeenCalledTimes(1));
    expect(confirm).not.toHaveBeenCalled();
    expect(saveGroup.mock.calls[0][0].udids).toEqual([]);
  });

  /// The header counts what is left, which is how "nhóm 2 là các máy chưa chọn" is read.
  it("says how many phones are still unassigned", () => {
    open([group("g1", "Nhóm 1", ["u-a", "u-e"])]);
    expect(screen.getByText(/1 máy chưa thuộc nhóm/)).toBeVisible();
  });

  /// Creating asks for nothing: groups are addressed by number, so the button names it.
  it("creates the next numbered group without asking for a name", async () => {
    open([]);
    fireEvent.click(screen.getByRole("button", { name: /Tạo Nhóm 1/ }));

    await waitFor(() => expect(saveGroup).toHaveBeenCalledTimes(1));
    const created = saveGroup.mock.calls[0][0];
    expect(created.name).toBe("Nhóm 1");
    expect(created.udids).toEqual([]);
    expect(created.id).toMatch(/^[A-Za-z0-9_-]+$/);
  });

  /// The next *free* number, not `length + 1`.
  ///
  /// Delete Nhóm 2 out of three and `length + 1` proposes Nhóm 3, which already exists — two
  /// rows with the same name, in a screen where the name is the only thing distinguishing
  /// them.
  it("fills the gap left by a deleted group instead of counting", async () => {
    open([group("g1", "Nhóm 1", []), group("g3", "Nhóm 3", [])]);
    fireEvent.click(screen.getByRole("button", { name: /Tạo Nhóm 2/ }));

    await waitFor(() => expect(saveGroup).toHaveBeenCalledTimes(1));
    expect(saveGroup.mock.calls[0][0].name).toBe("Nhóm 2");
  });

  /// **A fleet nobody has numbered still shows numbers.**
  ///
  /// Reported from the running app: with no `Change Number` anywhere, every chip rendered as
  /// `— SM G955F`, which is twenty identical buttons on a fleet of twenty identical phones.
  /// The wall numbers its tiles by position for this reason and this screen has to agree with
  /// it, or the numbers an operator reads off the wall address nothing here.
  it("numbers an unnumbered fleet by position, like the wall does", () => {
    const onChanged = vi.fn();
    render(
      <GroupManagerPopup
        devices={devices}
        groups={[group("g1", "Nhóm 1", [])]}
        metas={new Map()}
        onChanged={onChanged}
        onClose={() => undefined}
      />,
    );
    const numbers = screen
      .getAllByRole("button")
      .map((element) => element.querySelector(".group-chip-num")?.textContent)
      .filter((value): value is string => Boolean(value));
    expect(numbers).toEqual(["1", "2", "3"]);
  });

  /// **A phone in the open group wears that group's colour, not one shared accent.**
  ///
  /// The rows above are recognised by their colour dot, so the phones inside a group have to
  /// carry the same colour — otherwise the two halves of this screen describe the same thing
  /// in two different languages.
  it("paints picked phones in the open group's colour", () => {
    const green = { ...group("g1", "Nhóm 1", ["u-a"]), color: "#22c55e" };
    open([green]);
    const chip = screen.getByTitle("A").closest(".group-chip") as HTMLElement;
    expect(chip.style.background).toBe("rgb(34, 197, 94)");
  });

  /// **No toast for picking a phone.**
  ///
  /// `.toast-host` is fixed to the bottom-right corner — where this panel sits — and stacks
  /// above it, so one toast per phone covered the list being worked down. The chip changing
  /// colour is the same news in the place it happened.
  it("does not raise a toast for every phone picked", async () => {
    const { pushToast } = await import("../toastStore");
    open([group("g1", "Nhóm 1", [])]);
    fireEvent.click(screen.getByTitle("A"));

    await waitFor(() => expect(saveGroup).toHaveBeenCalledTimes(1));
    expect(pushToast).not.toHaveBeenCalled();
  });

  /// Deleting asks, and says what happens to the phones — nothing.
  it("asks before deleting a group and then deletes it", async () => {
    open([group("g1", "Nhóm 1", ["u-a"])]);
    fireEvent.click(screen.getByRole("button", { name: "Xoá" }));

    await waitFor(() => expect(confirm).toHaveBeenCalledTimes(1));
    expect(confirm.mock.calls[0][0]).toMatchObject({ danger: true });
    await waitFor(() => expect(deleteGroup).toHaveBeenCalledWith("g1"));
  });
});
