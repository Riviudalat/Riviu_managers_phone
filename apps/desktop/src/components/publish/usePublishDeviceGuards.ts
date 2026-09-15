import { useCallback, useEffect, useRef, useState } from "react";
import { publishDeviceGuards } from "../../api";
import type { PublishDeviceGuards } from "../../types";

/** One read at a time across roster changes/events; batches never exceed 500 IDs. */
export function usePublishDeviceGuards(udids: string[]) {
  const key = JSON.stringify([...new Set(udids)].sort());
  const roster = useRef<string[]>([]);
  roster.current = JSON.parse(key) as string[];
  const latestKey = useRef(key); latestKey.current = key;
  const [guards, setGuards] = useState<PublishDeviceGuards>({});
  const [reportedKey, setReportedKey] = useState("");
  const [failed, setFailed] = useState(false);
  const mounted = useRef(false), running = useRef(false), again = useRef(false);
  const revision = useRef(0);
  const refresh = useCallback(async () => {
    if (!mounted.current) return;
    revision.current += 1;
    if (running.current) { again.current = true; return; }
    running.current = true;
    try {
      do {
        again.current = false;
        const requestKey = latestKey.current, requestRevision = revision.current, ids = [...roster.current];
        try {
          const values: PublishDeviceGuards = {};
          for (let offset = 0; offset < ids.length; offset += 500) {
            const batch = ids.slice(offset, offset + 500);
            const response = await publishDeviceGuards(batch);
            if (!mounted.current || latestKey.current !== requestKey || revision.current !== requestRevision) break;
            for (const id of batch) {
              const guard = response[id];
              if (!guard || !Array.isArray(guard.blocking) || !Array.isArray(guard.linkReview)) throw Error("Incomplete device guard response");
              values[id] = guard;
            }
          }
          if (mounted.current && latestKey.current === requestKey && revision.current === requestRevision) { setGuards(values); setReportedKey(requestKey); setFailed(false); }
        } catch {
          if (mounted.current && latestKey.current === requestKey && revision.current === requestRevision) { setGuards({}); setFailed(true); }
        }
      } while (mounted.current && again.current);
    } finally { running.current = false; }
  }, []);
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; again.current = false; };
  }, []);
  useEffect(() => { void refresh(); }, [key, refresh]);
  useEffect(() => {
    const interval = setInterval(() => { if (document.visibilityState !== "hidden" && !running.current) void refresh(); }, 5000);
    const focus = () => { void refresh(); };
    window.addEventListener("focus", focus);
    return () => { clearInterval(interval); window.removeEventListener("focus", focus); };
  }, [refresh]);
  return { guards: reportedKey === key ? guards : {}, failed, refresh };
}
