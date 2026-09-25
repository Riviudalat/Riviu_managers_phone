import { QueryClient } from "@tanstack/react-query";

/** Only persisted read models belong here. Device probes and effects stay in api.ts. */
export const readQueryClient = new QueryClient({
  defaultOptions: { queries: {
    retry: false, refetchOnWindowFocus: false, refetchOnReconnect: false,
    staleTime: 0, gcTime: 60_000,
  } },
});

/** Mark one persisted read-model family stale after its owning mutation succeeds. */
export async function invalidateReadScope(scope: readonly unknown[]) {
  await readQueryClient.invalidateQueries({ queryKey: scope });
}
