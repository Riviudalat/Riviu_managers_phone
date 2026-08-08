import { useEffect, useState } from "react";

import type { DeviceGroup, DeviceInfo } from "../types";
import { listGroups } from "../api";
import { pushToast, toastError } from "../toastStore";

/** Shared selection strip so Publish/Apps/Jobs/etc. are usable without mystery disabled buttons. */
export function SelectionStrip({
  devices,
  selected,
  onSelectAll,
  onClear,
  onSelectUdids,
}: {
  devices: DeviceInfo[];
  selected: string[];
  onSelectAll: () => void;
  onClear: () => void;
  /** When provided, a "pick a group" dropdown loads it into the selection. */
  onSelectUdids?: (udids: string[]) => void;
}) {
  const [groups, setGroups] = useState<DeviceGroup[]>([]);
  useEffect(() => {
    if (!onSelectUdids) return;
    let alive = true;
    listGroups()
      .then((next) => {
        if (alive) setGroups(next);
      })
      .catch(() => {
        /* groups are a convenience; ignore load failures here */
      });
    return () => {
      alive = false;
    };
  }, [onSelectUdids]);

  const n = selected.length || devices.length;
  const usingAll = selected.length === 0 && devices.length > 0;
  return (
    <div className="selection-strip">
      <span>
        {usingAll ? (
          <>
            Chưa chọn → sẽ dùng <strong>tất cả {devices.length}</strong> máy
          </>
        ) : (
          <>
            Đang chọn <strong>{selected.length}</strong> / {devices.length} máy
          </>
        )}
      </span>
      <div className="grow" />
      {onSelectUdids && groups.length > 0 && (
        <select
          className="ghost"
          aria-label="Chọn theo nhóm"
          value=""
          onChange={(event) => {
            const group = groups.find((candidate) => candidate.id === event.currentTarget.value);
            if (group) onSelectUdids(group.udids);
          }}
        >
          <option value="">Chọn nhóm…</option>
          {groups.map((group) => (
            <option key={group.id} value={group.id}>
              {group.name} ({group.udids.length})
            </option>
          ))}
        </select>
      )}
      <button type="button" className="ghost" disabled={!devices.length} onClick={onSelectAll}>
        Chọn tất cả
      </button>
      <button type="button" className="ghost" disabled={!selected.length} onClick={onClear}>
        Bỏ chọn
      </button>
      {!devices.length && <span className="hint">Chưa có thiết bị — về Quản lý cửa sổ → Refresh</span>}
      {!!devices.length && (
        <span className="hint mono" title={selected.join(", ") || devices.map((d) => d.udid).join(", ")}>
          target×{n}
        </span>
      )}
    </div>
  );
}

export function targetsOf(selected: string[], devices: DeviceInfo[]): string[] {
  return selected.length ? selected : devices.map((d) => d.udid);
}

/**
 * Report the outcome of a farm action. Single choke point for the farm pages,
 * so they all notify through the in-app toast stack instead of an OS dialog.
 */
export function flash(msg: string) {
  const [title, ...rest] = msg.split("\n");
  pushToast("info", title, rest.join("\n") || undefined);
}

/** Report a failed farm action; normalises Tauri/Error/string throwables. */
export function flashError(cause: unknown, title = "Thao tác thất bại") {
  toastError(title, cause);
}
