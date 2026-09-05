import { useCallback, useEffect, useRef, useState } from "react";
import { operationGetRun, operationQueryRuns } from "./api";
import { describeError } from "./describeError";
import type { OperationRunDetail } from "./types";

export function useLibraryBatch(kind: "appInstall" | "materialTransfer") {
  const [detail, setDetail] = useState<OperationRunDetail | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const request = useRef({ ticket: 0, reading: false });
  const reload = useCallback(async () => {
    if (request.current.reading) return;
    request.current.reading = true;
    const current = ++request.current.ticket;
    try {
      const page = await operationQueryRuns({ kind, limit: 1, since: new Date(Date.now() - 86400000).toISOString() });
      const next = page.runs[0] ? await operationGetRun(page.runs[0].id) : null;
      if (current !== request.current.ticket) return;
      setDetail(next);
      setError(null);
    } catch (cause) {
      if (current === request.current.ticket) setError(describeError(cause));
    } finally {
      if (current === request.current.ticket) {
        request.current.reading = false;
        setLoading(false);
      }
    }
  }, [kind]);
  useEffect(() => {
    const identity = request.current;
    void reload();
    const timer = window.setInterval(() => void reload(), 2000);
    return () => { ++identity.ticket; identity.reading = false; window.clearInterval(timer); };
  }, [reload]);
  const active = detail?.items.some((item) => item.state === "queued" || item.state === "running") ?? false;
  return { detail, loading, error, active, reload };
}
