import { useEffect, useRef } from "react";
import { agentPreflight } from "./api";
import type { AgentStatus, DeviceInfo } from "./types";

/** At most this many phones are checked at once, so a fleet check never floods adb. */
const CONCURRENCY = 2;
/** A phone that is still not proved is checked again after this long, never in a tight loop. */
const RETRY_MS = 5 * 60_000;

function proved(status: AgentStatus | undefined): boolean {
  return Boolean(
    status && status.state === "ready" && status.authReady && status.sessionReady && status.features.includes("helperReady"),
  );
}

/**
 * Checks control of every idle, connected Android phone that is not proved yet ("Chưa kiểm"),
 * so the wall reaches "Sẵn sàng" by itself (owner request: "phải luôn kiểm tra").
 *
 * `agent_preflight` takes the phone with `try_acquire_exclusive`, so a phone a job holds is
 * skipped, never taken over; it checks the helper and the control session and touches no
 * TikTok account. Each phone is attempted at most once per `RETRY_MS`.
 */
export function useAutoControlCheck(options: {
  enabled: boolean;
  devices: readonly DeviceInfo[];
  cachedAgent: (udid: string) => AgentStatus | undefined;
  isBusy: (udid: string) => boolean;
}) {
  const { enabled, devices, cachedAgent, isBusy } = options;
  const lastAttempt = useRef(new Map<string, number>());
  const running = useRef(new Set<string>());

  useEffect(() => {
    if (!enabled) return;
    let cancelled = false;
    const tick = () => {
      if (cancelled) return;
      const now = Date.now();
      const due = devices.filter((device) =>
        device.platform === "android"
        && (device.status === "connected" || device.status === "ready")
        && !running.current.has(device.udid)
        && !isBusy(device.udid)
        && !proved(cachedAgent(device.udid))
        && now - (lastAttempt.current.get(device.udid) ?? 0) >= RETRY_MS,
      );
      for (const device of due) {
        if (running.current.size >= CONCURRENCY) break;
        running.current.add(device.udid);
        lastAttempt.current.set(device.udid, now);
        void agentPreflight(device.udid)
          .catch(() => undefined) // a busy or failing phone keeps its reason in the cached status
          .finally(() => {
            running.current.delete(device.udid);
            if (!cancelled) tick();
          });
      }
    };
    tick();
    const timer = window.setInterval(tick, 15_000);
    return () => { cancelled = true; window.clearInterval(timer); };
  }, [enabled, devices, cachedAgent, isBusy]);
}
