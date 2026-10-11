import { useCallback, useEffect, useMemo, useState } from "react";
import { automationGet, listGroups } from "./api";
import { requestConfirm } from "./confirmStore";
import { describeError } from "./describeError";
import type { AutomationSchedule, DeviceGroup, TargetRef } from "./types";

export interface DeviceGroupsRead {
  /** `null` while reading or after a failed read: a group target is then unknown, never approved. */
  groups: DeviceGroup[] | null;
  error: string | null;
  reload: () => void;
}

/**
 * Groups for resolving group targets, read only once some target is a group, and read again
 * whenever `refreshToken` changes (each schedule refresh) or `reload` is called. A reread starts
 * from unknown: membership read before a refresh is not evidence for after it, so a group
 * changed elsewhere shows its difference instead of the old approval.
 */
export function useDeviceGroups(needed: boolean, refreshToken?: unknown): DeviceGroupsRead {
  const [read, setRead] = useState<{ groups: DeviceGroup[] | null; error: string | null }>({ groups: null, error: null });
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    if (!needed) return;
    let active = true;
    setRead({ groups: null, error: null });
    listGroups()
      .then((next) => { if (active) setRead({ groups: next, error: null }); })
      .catch((cause) => { if (active) setRead({ groups: null, error: describeError(cause) }); });
    return () => { active = false; };
  }, [needed, refreshToken, attempt]);
  const reload = useCallback(() => setAttempt((value) => value + 1), []);
  return { ...read, reload };
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

/** The clock a preview reads; "if switched on now" moves with it, so a page left open stays true. */
export function usePreviewClock() {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 30_000);
    return () => window.clearInterval(timer);
  }, []);
  return now;
}
