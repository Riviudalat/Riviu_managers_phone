import { useEffect, useMemo, useState } from "react";
import { automationGet, listGroups } from "./api";
import { requestConfirm } from "./confirmStore";
import type { AutomationSchedule, DeviceGroup, TargetRef } from "./types";

/**
 * Groups for resolving group targets, read only once some target is a group. `null` until
 * read, so a group target is never guessed empty.
 */
export function useDeviceGroups(needed: boolean): DeviceGroup[] | null {
  const [groups, setGroups] = useState<DeviceGroup[] | null>(null);
  useEffect(() => {
    if (!needed) return;
    let active = true;
    listGroups()
      .then((next) => { if (active) setGroups(next); })
      // Without groups only group targets stay unknown; "all" and explicit targets still resolve.
      .catch(() => undefined);
    return () => { active = false; };
  }, [needed]);
  return groups;
}

/** Whether any of these targets needs the group list to resolve. */
export function needsGroups(targets: Iterable<TargetRef | null | undefined>) {
  for (const target of targets) if (target?.type === "group") return true;
  return false;
}

/** The target each schedule's pinned profile revision names, keyed by `definitionId:revision`. */
export function useScheduleTargets(rows: Pick<AutomationSchedule, "definitionId" | "definitionRevision">[]) {
  const keys = useMemo(
    () => [...new Set(rows.map((row) => `${row.definitionId}:${row.definitionRevision}`))].sort(),
    [rows],
  );
  const keyList = keys.join("|");
  const [targets, setTargets] = useState<Map<string, TargetRef | null>>(new Map());
  useEffect(() => {
    let active = true;
    void Promise.all(keyList ? keyList.split("|").map(async (key) => {
      const [definitionId, revision] = key.split(":");
      try {
        const record = await automationGet(definitionId, Number(revision));
        return [key, record?.revision.targetRef ?? null] as const;
      } catch {
        return [key, undefined] as const;
      }
    }) : []).then((entries) => {
      if (!active) return;
      setTargets(new Map(entries.filter((entry): entry is readonly [string, TargetRef | null] => entry[1] !== undefined)));
    });
    return () => { active = false; };
  }, [keyList]);
  return targets;
}

export function deviceNames(udids: string[], label: (udid: string) => string) {
  return udids.map(label).join(", ");
}

/** Asks before storing a new approved list; the dialog names every phone it covers. */
export async function confirmScheduleApproval(name: string, udids: string[], label: (udid: string) => string) {
  return requestConfirm({
    title: `Duyệt ${udids.length} máy cho lịch ${name}?`,
    message: `${deviceNames(udids, label)}. Lịch chỉ chạy trên các máy này. Nhóm đổi sau đó sẽ dừng lịch tới khi duyệt lại; lịch không tự thêm máy.`,
    confirmLabel: `Duyệt ${udids.length} máy`,
    cancelLabel: "Chưa duyệt",
  });
}
