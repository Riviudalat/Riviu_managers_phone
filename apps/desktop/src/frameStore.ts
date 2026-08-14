import { useEffect, useSyncExternalStore } from "react";

type Listener = () => void;

const latest = new Map<string, string>();
const pending = new Map<string, string>();
const perDevice = new Map<string, Set<Listener>>();
let flushHandle: number | ReturnType<typeof setTimeout> | null = null;

function emit(udid: string) {
  const set = perDevice.get(udid);
  if (!set) return;
  for (const l of set) l();
}

function flushPending() {
  flushHandle = null;
  for (const [deviceUdid, frame] of pending) {
    pending.delete(deviceUdid);
    if (latest.get(deviceUdid) === frame) continue;
    latest.set(deviceUdid, frame);
    emit(deviceUdid);
  }
  if (pending.size > 0) scheduleFlush();
}

function scheduleFlush() {
  if (flushHandle !== null) return;
  if (typeof window !== "undefined" && typeof window.requestAnimationFrame === "function") {
    flushHandle = window.requestAnimationFrame(flushPending);
  } else {
    flushHandle = setTimeout(flushPending, 16);
  }
}

/** Push a JPEG base64 frame for Flow / evidence hydrate. Tiles and the
 *  overlay no longer read this — they paint from the view worker. */
export function pushFrame(udid: string, jpegBase64: string) {
  if (!udid || !jpegBase64) return;
  // Coalesce bursts from Tauri into one commit per browser frame. The Rust
  // scheduler already bounds total events; this last gate keeps image decode
  // and React notifications aligned with the WebView paint loop.
  if (latest.get(udid) === jpegBase64 || pending.get(udid) === jpegBase64) return;
  pending.set(udid, jpegBase64);
  scheduleFlush();
}

export function peekFrame(udid: string): string | undefined {
  return latest.get(udid);
}

/**
 * Recover a frame that arrived before the WebView subscribed to the event
 * bus. The backend keeps the last decoded JPEG in StreamHub, so this is a
 * cheap local read and stops retrying as soon as the cache is hydrated.
 */
export function useHydratedDeviceFrame(
  udid: string,
  loadFrame: (udid: string) => Promise<string | null>,
) {
  useEffect(() => {
    let cancelled = false;
    let retry: ReturnType<typeof setTimeout> | undefined;

    const hydrate = async () => {
      if (peekFrame(udid)) return;
      try {
        const frame = await loadFrame(udid);
        if (!cancelled && frame) pushFrame(udid, frame);
      } catch {
        // Device discovery and stream startup are asynchronous; retry below.
      }
      if (!cancelled && !peekFrame(udid)) {
        retry = setTimeout(() => void hydrate(), 750);
      }
    };

    void hydrate();
    return () => {
      cancelled = true;
      if (retry) clearTimeout(retry);
    };
  }, [loadFrame, udid]);
}

/**
 * Subscribe one device's stream. Only this component re-renders on new frames
 * (grid siblings stay idle → smoother + mini stays live while focus is open).
 */
export function useDeviceFrame(udid: string): string | undefined {
  return useSyncExternalStore(
    (onStoreChange) => {
      let set = perDevice.get(udid);
      if (!set) {
        set = new Set();
        perDevice.set(udid, set);
      }
      set.add(onStoreChange);
      return () => {
        set!.delete(onStoreChange);
        if (set!.size === 0) perDevice.delete(udid);
      };
    },
    () => latest.get(udid),
    () => undefined,
  );
}
