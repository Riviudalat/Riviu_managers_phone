import { useMemo, useState } from "react";

import { deleteGroup, saveGroup } from "../api";
import { requestConfirm } from "../confirmStore";
import { orderDevicesByNumber, tileName, tileNumber } from "../deviceNaming";
import type { DeviceGroup, DeviceInfo, DeviceMeta } from "../types";
import { toastError } from "../toastStore";
import { IconClose } from "./Icons";

/**
 * Divide the fleet into groups: nhóm 1 is these phones, nhóm 2 is some of what is left.
 *
 * **Membership is exclusive**, and the picker shows that rather than explaining it: a phone
 * already in another group is drawn with that group's colour and, when picked, moves. The
 * database enforces it in the same transaction as the write (`write_group`), so this UI never
 * has to hold two saves together to keep the rule.
 *
 * Phones are listed and stored **by the operator's number**, because that is what the whole
 * feature is addressed in — "nhóm 1: máy 1, 2, 5". The number also decides the order inside a
 * group, and that order is not cosmetic: the interaction panel loads a group straight into its
 * actor list, and a `Chain` campaign replies down that list in order. Group A of 1, 4, 2
 * therefore always runs 1 → 2 → 4, which is predictable; before this it ran in whatever order
 * the SQLite query plan produced, which was by serial number and looked random.
 */

const COLORS = ["#f97316", "#0ea5e9", "#22c55e", "#a855f7", "#ef4444", "#eab308"];

function newGroupId(): string {
  return `g-${crypto.randomUUID().slice(0, 8)}`;
}

export function GroupManagerPopup({
  devices,
  groups,
  metas,
  onChanged,
  onClose,
}: {
  devices: DeviceInfo[];
  groups: DeviceGroup[];
  metas: Map<string, DeviceMeta>;
  /** Reload the fleet after a write, so the tab strip and every picker see it. */
  onChanged: () => Promise<void> | void;
  onClose: () => void;
}) {
  const [openId, setOpenId] = useState<string | null>(groups[0]?.id ?? null);
  const [busy, setBusy] = useState(false);

  /// Every phone with the number the wall shows it under, in that order.
  ///
  /// **`tileNumber`, which falls back to the grid position.** The first version printed "—"
  /// for a phone the operator had never numbered, on the theory that borrowing a position
  /// would read as a real number. On this fleet nobody has numbered anything, so every chip
  /// came out `— SM G955F` — twenty identical buttons, and no way to tell which phone was
  /// which. The wall has always numbered its tiles 1..20 by position for exactly this reason,
  /// and a screen that disagreed with the wall would be worse than one that borrows.
  const rows = useMemo(
    () =>
      orderDevicesByNumber(devices, metas).map((device, index) => ({
        udid: device.udid,
        label: tileName(device, metas.get(device.udid)),
        number: tileNumber(index + 1, metas.get(device.udid)),
      })),
    [devices, metas],
  );

  /// Which group each phone is in right now, for the colour dot and the move warning.
  const groupOf = useMemo(() => {
    const map = new Map<string, DeviceGroup>();
    for (const group of groups) {
      for (const udid of group.udids) map.set(udid, group);
    }
    return map;
  }, [groups]);

  const open = groups.find((group) => group.id === openId) ?? null;
  const unassigned = rows.filter((row) => !groupOf.has(row.udid));

  /// Write a group, then let the app reload — every list of groups is derived from the fleet
  /// poll, so nothing here keeps its own copy to drift.
  ///
  /// **No toast on success.** `.toast-host` is `position: fixed; right: 1rem; bottom: 1rem`,
  /// which is where this panel sits, and it stacks above it — so a toast for every phone
  /// picked covered the very list the operator was working down. Nothing is lost: the chip
  /// changes colour and the counts move, which is the same news arriving in the place it
  /// happened. Failures still speak, because a write that did not happen leaves no other mark.
  const write = async (group: DeviceGroup) => {
    setBusy(true);
    try {
      await saveGroup(group);
      await onChanged();
    } catch (error) {
      toastError("Lưu nhóm thất bại", error);
    } finally {
      setBusy(false);
    }
  };

  /// The next free `Nhóm N`.
  ///
  /// Not `groups.length + 1`: delete nhóm 2 of three and that would propose `Nhóm 3`, which
  /// already exists. Groups are only ever addressed by their number here, so a duplicate name
  /// is two rows the operator cannot tell apart.
  const nextName = useMemo(() => {
    const taken = new Set(groups.map((group) => group.name));
    let n = 1;
    while (taken.has(`Nhóm ${n}`)) n += 1;
    return `Nhóm ${n}`;
  }, [groups]);

  const create = async () => {
    const group: DeviceGroup = {
      id: newGroupId(),
      name: nextName,
      color: COLORS[groups.length % COLORS.length],
      udids: [],
      createdAt: new Date().toISOString(),
    };
    setOpenId(group.id);
    await write(group);
  };

  /// Add or remove one phone, keeping the group in number order.
  const toggle = async (udid: string) => {
    if (!open) return;
    const inGroup = open.udids.includes(udid);
    const owner = groupOf.get(udid);
    if (!inGroup && owner && owner.id !== open.id) {
      const moved = await requestConfirm({
        title: `Chuyển máy sang ${open.name}?`,
        message: `Máy này đang ở nhóm ${owner.name}. Một máy chỉ thuộc một nhóm, nên thêm vào đây là bỏ khỏi ${owner.name}.`,
        confirmLabel: "Chuyển",
        cancelLabel: "Để nguyên",
      });
      if (!moved) return;
    }
    const next = inGroup
      ? open.udids.filter((id) => id !== udid)
      : [...open.udids, udid];
    // Sorted on the way in, so what is stored is what runs: the order decides who replies to
    // whom in a Chain, and "the order I happened to click" is not something the operator can
    // see afterwards.
    const order = new Map(rows.map((row, index) => [row.udid, index]));
    next.sort((a, b) => (order.get(a) ?? 0) - (order.get(b) ?? 0));
    await write({ ...open, udids: next });
  };

  const remove = async (group: DeviceGroup) => {
    const yes = await requestConfirm({
      title: `Xoá nhóm ${group.name}?`,
      message: `${group.udids.length} máy trong nhóm sẽ trở lại "chưa thuộc nhóm nào". Máy không bị đụng gì.`,
      confirmLabel: "Xoá nhóm",
      cancelLabel: "Giữ lại",
      danger: true,
    });
    if (!yes) return;
    setBusy(true);
    try {
      await deleteGroup(group.id);
      await onChanged();
      if (openId === group.id) setOpenId(null);
    } catch (error) {
      toastError("Xoá nhóm thất bại", error);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="nurture-float-layer">
      <section className="nurture-float group-manager" aria-label="Quản lý nhóm">
        <header className="nurture-float-title">
          <strong>Nhóm máy</strong>
          <span className="hint">
            {groups.length} nhóm · {unassigned.length} máy chưa thuộc nhóm
          </span>
          <div className="grow" />
          <button type="button" className="close" title="Đóng" onClick={onClose}>
            <IconClose size={14} />
          </button>
        </header>

        <div className="nurture-float-body">
          {/* No name field: groups are addressed by number and nothing else, so asking for
              one was a box to fill in before the screen would do anything. */}
          <div className="group-new">
            <button type="button" className="ghost" disabled={busy} onClick={() => void create()}>
              + Tạo {nextName}
            </button>
          </div>

          {groups.length === 0 && (
            <p className="nu-hint">
              Chưa có nhóm nào. Tạo một nhóm rồi bấm các máy để cho vào — mỗi máy chỉ thuộc
              một nhóm, nên nhóm sau chỉ còn những máy chưa được chọn.
            </p>
          )}

          {groups.map((group) => (
            <div key={group.id} className="group-row">
              <button
                type="button"
                className={`group-head${group.id === openId ? " is-on" : ""}`}
                style={group.id === openId ? { borderColor: group.color } : undefined}
                aria-expanded={group.id === openId}
                onClick={() => setOpenId(group.id === openId ? null : group.id)}
              >
                <span className="device-menu-dot" style={{ background: group.color }} />
                <strong>{group.name}</strong>
                <span className="hint">{group.udids.length} máy</span>
              </button>
              <button
                type="button"
                className="ghost"
                disabled={busy}
                onClick={() => void remove(group)}
              >
                Xoá
              </button>
            </div>
          ))}

          {open && (
            <div className="group-pick">
              <span className="hint">
                Bấm để thêm hoặc bỏ máy khỏi <strong>{open.name}</strong>. Máy đang ở nhóm
                khác có chấm màu của nhóm đó.
              </span>
              <div className="group-pick-grid">
                {rows.map((row) => {
                  const owner = groupOf.get(row.udid);
                  const mine = open.udids.includes(row.udid);
                  return (
                    <button
                      key={row.udid}
                      type="button"
                      className={`group-chip${mine ? " selected" : ""}`}
                      // The open group's own colour, not one shared accent: the dots on the
                      // rows above are how a group is recognised, so the phones in it have to
                      // carry the same colour or the two halves of this screen do not match.
                      style={mine ? { background: open.color, borderColor: open.color } : undefined}
                      disabled={busy}
                      title={
                        owner && owner.id !== open.id
                          ? `${row.label} — đang ở nhóm ${owner.name}`
                          : row.label
                      }
                      onClick={() => void toggle(row.udid)}
                    >
                      {owner && owner.id !== open.id && (
                        <span
                          className="device-menu-dot"
                          style={{ background: owner.color }}
                        />
                      )}
                      <span className="group-chip-num">{row.number}</span>
                    </button>
                  );
                })}
              </div>
            </div>
          )}
        </div>
      </section>
    </div>
  );
}
