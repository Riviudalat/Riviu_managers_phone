import { useCallback, useEffect, useRef, useState } from "react";

import type { DeviceGroup, DeviceInfo } from "../types";
import { listGroups } from "../api";
import { describeError } from "../describeError";
import { LoadingState, StatusNotice } from "./States";

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
  const [groupState, setGroupState] = useState<"idle" | "loading" | "ready" | "error">("idle");
  const [groupError, setGroupError] = useState<string | null>(null);
  const groupLoadTicket = useRef(0);
  const loadGroups = useCallback(async () => {
    if (!onSelectUdids) return;
    const ticket = ++groupLoadTicket.current;
    setGroupState("loading");
    setGroupError(null);
    try {
      const next = await listGroups();
      if (ticket !== groupLoadTicket.current) return;
      setGroups(next);
      setGroupState("ready");
    } catch (error) {
      if (ticket !== groupLoadTicket.current) return;
      setGroups([]);
      setGroupError(describeError(error));
      setGroupState("error");
    }
  }, [onSelectUdids]);

  useEffect(() => {
    if (!onSelectUdids) {
      groupLoadTicket.current += 1;
      setGroups([]);
      setGroupState("idle");
      return;
    }
    void loadGroups();
    return () => {
      groupLoadTicket.current += 1;
    };
  }, [loadGroups, onSelectUdids]);

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
            if (group?.udids.length) onSelectUdids(group.udids);
          }}
        >
          <option value="">Chọn nhóm…</option>
          {groups.map((group) => (
            <option key={group.id} value={group.id} disabled={!group.udids.length}>
              {group.name} ({group.udids.length})
            </option>
          ))}
        </select>
      )}
      {onSelectUdids && groupState === "loading" && <LoadingState label="Đang tải nhóm…" />}
      {onSelectUdids && groupState === "error" && (
        <StatusNotice
          tone="error"
          action={<button type="button" className="ghost" onClick={() => void loadGroups()}>Thử lại nhóm</button>}
        >
          {groupError ?? "Không tải được nhóm thiết bị."}
        </StatusNotice>
      )}
      <button type="button" className="ghost" disabled={!devices.length} onClick={onSelectAll}>
        Chọn tất cả
      </button>
      <button type="button" className="ghost" disabled={!selected.length} onClick={onClear}>
        Bỏ chọn
      </button>
      {!devices.length && (
        <span className="hint">Chưa có thiết bị kết nối</span>
      )}
      {!!devices.length && (
        <span className="hint">Phạm vi: {n} máy</span>
      )}
    </div>
  );
}
