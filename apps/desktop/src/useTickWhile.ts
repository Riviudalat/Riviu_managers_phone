import { useEffect, useState } from "react";

/** How often the bars re-read the clock while anything is running. */
const TICK_MS = 1_000;

/**
 * A clock that ticks only while it is needed.
 *
 * The bar's fill depends on `Date.now()` — one of the two bounds is a wall clock — so a bar
 * driven purely by status events would freeze for the twenty seconds a phone spends watching
 * one video and then jump. Stopped when nothing is running, so a panel left open on a
 * finished run does not re-render once a second forever.
 */
export function useTickWhile(active: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    setNow(Date.now());
    const timer = setInterval(() => setNow(Date.now()), TICK_MS);
    return () => clearInterval(timer);
  }, [active]);
  return now;
}

