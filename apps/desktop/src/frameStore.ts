import { useSyncExternalStore } from "react";

type Listener = () => void;

const latest = new Map<string, string>();
const perDevice = new Map<string, Set<Listener>>();

function emit(udid: string) {
  const set = perDevice.get(udid);
  if (!set) return;
  for (const l of set) l();
}

/** Push a JPEG base64 frame from the Tauri stream event bus. */
export function pushFrame(udid: string, jpegBase64: string) {
  if (!udid || !jpegBase64) return;
  // Skip identical payload to avoid needless paint storms
  if (latest.get(udid) === jpegBase64) return;
  latest.set(udid, jpegBase64);
  emit(udid);
}

export function peekFrame(udid: string): string | undefined {
  return latest.get(udid);
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
