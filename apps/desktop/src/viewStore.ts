import { useEffect, useSyncExternalStore } from "react";
import { viewEndpoint } from "./api";

export interface ViewSize {
  width: number;
  height: number;
  generation: number;
}

type Listener = () => void;

const sizes = new Map<string, ViewSize>();
const live = new Set<string>();
const listeners = new Map<string, Set<Listener>>();
const pendingExports = new Map<number, (bytes: Uint8Array | null) => void>();

export const VIEW_RECONNECT_MIN_MS = 200;
export const VIEW_RECONNECT_MAX_MS = 2000;

export function nextViewReconnectDelay(currentMs: number): number {
  return Math.min(Math.max(currentMs, VIEW_RECONNECT_MIN_MS) * 2, VIEW_RECONNECT_MAX_MS);
}

let worker: Worker | null = null;
let socket: WebSocket | null = null;
let started = false;
let exportId = 1;
let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
let reconnectDelayMs = VIEW_RECONNECT_MIN_MS;
let connecting = false;

function emit(udid: string) {
  const set = listeners.get(udid);
  if (!set) return;
  for (const listener of set) listener();
}

function ensureWorker(): Worker | null {
  if (worker) return worker;
  if (typeof Worker === "undefined") return null;
  try {
    worker = new Worker(new URL("./viewDecode.worker.ts", import.meta.url), { type: "module" });
  } catch {
    return null;
  }
  worker.onmessage = (event: MessageEvent<{ type: string; udid?: string; width?: number; height?: number; generation?: number; requestId?: number; bytes?: Uint8Array | null }>) => {
    const message = event.data;
    if (message.type === "painted" && message.udid) {
      const next: ViewSize = {
        width: message.width ?? 0,
        height: message.height ?? 0,
        generation: message.generation ?? 0,
      };
      const prev = sizes.get(message.udid);
      const same =
        prev !== undefined &&
        prev.width === next.width &&
        prev.height === next.height &&
        prev.generation === next.generation;
      const wasLive = live.has(message.udid);
      if (!same) sizes.set(message.udid, next);
      live.add(message.udid);
      if (!same || !wasLive) emit(message.udid);
      return;
    }
    if (message.type === "exportResult" && message.requestId != null) {
      pendingExports.get(message.requestId)?.(message.bytes ?? null);
      pendingExports.delete(message.requestId);
    }
  };
  return worker;
}

function scheduleReconnect() {
  // Test mode stays a single attempt so Vitest does not leak open timers.
  if (import.meta.env.MODE === "test") return;
  if (reconnectTimer != null) return;
  const delay = reconnectDelayMs;
  reconnectDelayMs = nextViewReconnectDelay(reconnectDelayMs);
  reconnectTimer = setTimeout(() => {
    reconnectTimer = null;
    void connectViewSocket();
  }, delay);
}

async function connectViewSocket() {
  if (connecting) return;
  connecting = true;
  try {
    const url = await viewEndpoint();
    if (!url) {
      scheduleReconnect();
      return;
    }
    connectSocket(url);
  } catch {
    scheduleReconnect();
  } finally {
    connecting = false;
  }
}

function connectSocket(url: string) {
  const previous = socket;
  const next = new WebSocket(url);
  next.binaryType = "arraybuffer";
  next.onopen = () => {
    reconnectDelayMs = VIEW_RECONNECT_MIN_MS;
  };
  next.onmessage = (event) => {
    if (!(event.data instanceof ArrayBuffer)) return;
    ensureWorker()?.postMessage({ type: "packet", buffer: event.data }, [event.data]);
  };
  next.onclose = () => {
    if (socket === next) {
      socket = null;
      scheduleReconnect();
    }
  };
  socket = next;
  previous?.close();
}

/** Open the loopback view WebSocket and keep one worker for every canvas. */
export function startViewClient() {
  if (started) return;
  started = true;
  ensureWorker();
  void connectViewSocket();
}

export function attachViewCanvas(udid: string, canvas: OffscreenCanvas, surfaceId: string) {
  ensureWorker()?.postMessage({ type: "attach", udid, surfaceId, canvas }, [canvas]);
}

export function detachViewCanvas(udid: string, surfaceId: string) {
  worker?.postMessage({ type: "detach", udid, surfaceId });
}

export function peekViewSize(udid: string): ViewSize | undefined {
  return sizes.get(udid);
}

export function peekViewLive(udid: string): boolean {
  return live.has(udid);
}

export function exportViewJpeg(udid: string): Promise<Uint8Array | null> {
  const target = ensureWorker();
  if (!target) return Promise.resolve(null);
  const requestId = exportId;
  exportId += 1;
  return new Promise((resolve) => {
    pendingExports.set(requestId, resolve);
    target.postMessage({ type: "export", udid, requestId });
  });
}

function subscribe(udid: string, onStoreChange: Listener) {
  let set = listeners.get(udid);
  if (!set) {
    set = new Set();
    listeners.set(udid, set);
  }
  set.add(onStoreChange);
  return () => {
    set!.delete(onStoreChange);
    if (set!.size === 0) listeners.delete(udid);
  };
}

export function useViewLive(udid: string): boolean {
  return useSyncExternalStore(
    (onStoreChange) => subscribe(udid, onStoreChange),
    () => live.has(udid),
    () => false,
  );
}

export function useViewSize(udid: string): ViewSize | undefined {
  return useSyncExternalStore(
    (onStoreChange) => subscribe(udid, onStoreChange),
    () => sizes.get(udid),
    () => undefined,
  );
}

export function useViewClient() {
  useEffect(() => {
    startViewClient();
  }, []);
}
