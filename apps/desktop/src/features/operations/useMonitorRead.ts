import { useId } from "react";
import { useQuery } from "@tanstack/react-query";
import { readQueryClient } from "../../readQuery";
import { describeError } from "../../describeError";

/** One in-flight read per mounted source. Changing selection cannot publish stale data. */
export function useMonitorRead<T>(read: () => Promise<T>, interval = 2000, scope?: readonly unknown[]) {
  const instance = useId();
  const query = useQuery({
    queryKey: ["monitor", ...(scope ?? [instance])], queryFn: read,
    refetchInterval: interval, refetchIntervalInBackground: true,
  }, readQueryClient);
  return { value: query.data ?? null, error: query.error ? describeError(query.error) : null,
    loading: query.isPending, retry: () => { void query.refetch(); } };
}
