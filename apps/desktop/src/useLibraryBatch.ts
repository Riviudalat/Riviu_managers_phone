import { useCallback, useEffect, useRef, useState } from "react";
import { operationGetRun, operationQueryRuns } from "./api";
import { describeError } from "./describeError";
import type { OperationRunDetail } from "./types";

export function useLibraryBatch(kind: "appInstall" | "materialTransfer", operationId?: string) {
  const [followed, setFollowed] = useState<{ origin?: string; id: string } | null>(null);
  const effectiveId = followed && followed.origin === operationId ? followed.id : operationId;
  const [detail, setDetail] = useState<OperationRunDetail | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const request = useRef({ ticket: 0, reading: false });
  const reload = useCallback(async () => {
    if (request.current.reading) return;
    request.current.reading = true;
    const current = ++request.current.ticket;
    try {
      const page = effectiveId ? null : await operationQueryRuns({ kind, limit: 1, since: new Date(Date.now() - 86400000).toISOString() });
      const id = effectiveId ?? page?.runs[0]?.id;
      const next = id ? await operationGetRun(id) : null;
      if (effectiveId && (!next || next.summary.id !== effectiveId || next.summary.kind !== kind)) {
        throw new Error("Lần chạy được chọn không còn trong nguồn dữ liệu.");
      }
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
  }, [kind, effectiveId]);
  useEffect(() => {
    const identity = request.current;
    setDetail(null);
    setLoading(true);
    setError(null);
    void reload();
    const timer = window.setInterval(() => void reload(), 2000);
    return () => { ++identity.ticket; identity.reading = false; window.clearInterval(timer); };
  }, [reload]);
  const active = detail?.items.some((item) => item.state === "queued" || item.state === "running") ?? false;
  const follow = useCallback((id: string) => setFollowed({ origin: operationId, id }), [operationId]);
  return { detail, loading, error, active, reload, follow };
}
