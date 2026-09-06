import { useEffect, useState } from "react";
import { describeError } from "../../describeError";

/** One in-flight read per mounted source. Changing selection cannot publish stale data. */
export function useMonitorRead<T>(read: () => Promise<T>, interval = 2000) {
  const [value, setValue] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [retry, setRetry] = useState(0);
  useEffect(() => {
    let disposed = false;
    let timer: number;
    setLoading(true);
    const load = async () => {
      try {
        const next = await read();
        if (!disposed) { setValue(next); setError(null); }
      } catch (cause) {
        if (!disposed) setError(describeError(cause));
      } finally {
        if (!disposed) { setLoading(false); timer = window.setTimeout(() => void load(), interval); }
      }
    };
    void load();
    return () => { disposed = true; window.clearTimeout(timer); };
  }, [read, interval, retry]);
  return { value, error, loading, retry: () => setRetry((count) => count + 1) };
}
