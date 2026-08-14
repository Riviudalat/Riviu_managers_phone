import { useEffect, useSyncExternalStore } from "react";
import { viewEndpoint, viewEnsure } from "./api";

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
/// When each udid last reported a drawn frame, and when we last acted on a stall.
/// The beat at which each udid last actually drew, and the newest beat of any kind.
const lastPaintBeat = new Map<string, ViewBeat>();
const latestBeat = new Map<string, ViewBeat>();
const lastRecoveryAt = new Map<string, number>();
/// How many restarts this udid has had without a single frame drawn since.
const recoveryAttempts = new Map<string, number>();
/// UDIDs whose every codec candidate was refused. Distinct from "not live": there is
/// nothing to wait for, so retrying the same stream cannot help.
const decodeFailed = new Set<string>();

/// A stream is stalled after this long of packets arriving that produce no drawn frame.
///
/// The predicate matters more than the number, and the first version of this got it wrong:
/// it treated "no frames drawn" as the fault. scrcpy only encodes when the screen changes,
/// so a phone parked on a static lock screen paints nothing for minutes and is perfectly
/// healthy -- the comment here said exactly that and the rule ignored it. Measured cost of
/// that mistake on a Redmi sitting on its lock screen: a restart every ~7 s of uptime
/// against ~45 s of spawn, leaving the stream up 14-18% of the time, and the restarts stole
/// the exclusive start claim so the overlay's own encode request came back
/// "already in flight" and never landed.
///
/// Arrivals climbing while paints stay flat is the condition that actually means broken, and
/// it needs both counters. 12s rather than 6: at 24 fps a healthy stream paints within a
/// frame or two of an arrival, so anything this long is not a slow decoder.
export const PAINT_STALL_MS = 12000;
/// Delay before the FIRST retry for one udid. Each further attempt doubles it.
///
/// A flat cooldown is not enough, and this was measured the hard way: a producer restart
/// takes about 44 s end to end on this fleet, so with a flat 20 s floor every stall
/// re-armed before the previous restart had even finished publishing, and a phone whose
/// frames could not be decoded was restarted forever -- roughly once a minute, each one
/// tearing down a working-but-undecodable stream. That is worse than the stale canvas it
/// was meant to replace.
export const PAINT_RECOVERY_COOLDOWN_MS = 30000;
/// How long a producer restart takes end to end, measured on this fleet (scrcpy push check,
/// leftover kill, forward, spawn, first keyframe). The backoff has to clear this by the
/// second retry or it re-arms mid-restart, which is the loop that was observed.
export const OBSERVED_RESTART_MS = 44000;
/// Ceiling on the backoff. Past this a device is checked occasionally rather than hammered;
/// a phone that has not painted in five minutes needs an operator, not another restart.
export const PAINT_RECOVERY_MAX_MS = 300000;
const STALL_TICK_MS = 2000;

let stallTimer: ReturnType<typeof setInterval> | null = null;

export function viewDecodeFailed(udid: string): boolean {
  return decodeFailed.has(udid);
}

/// One udid's most recent worker beat: how many envelopes arrived and how many frames were
/// drawn, as of `at`.
export interface ViewBeat {
  at: number;
  received: number;
  frames: number;
}

/// The udids that look stalled: packets kept arriving and no frame was drawn for them.
///
/// Takes its inputs rather than reading module state, so the policy can be tested without a
/// worker, a socket, or a timer. That matters here more than usual: the defect this exists
/// for was invisible precisely because nothing observable changed, and a rule that can only
/// be exercised by running the whole app is a rule that goes untested.
///
/// A udid with no recorded paint time is NOT stalled. It has never drawn, so it is either
/// still starting up or already reported through another path, and treating "never" as
/// "stalled" would restart every stream the moment it appears.
export function collectStalledViews(
  now: number,
  liveUdids: Iterable<string>,
  lastPaint: Map<string, ViewBeat>,
  latest: Map<string, ViewBeat>,
  stallMs: number = PAINT_STALL_MS,
): string[] {
  const stalled: string[] = [];
  for (const udid of liveUdids) {
    const painted = lastPaint.get(udid);
    const now_ = latest.get(udid);
    // Never drawn, or no beat at all: starting up, not stalled. Restarting a producer the
    // instant its device appears is how the previous rule made the outage it was reporting.
    if (painted === undefined || now_ === undefined) continue;
    if (now - painted.at <= stallMs) continue;
    // The whole point: only a stream whose packets kept coming is broken. A static screen
    // stops producing packets too, and restarting it fixes nothing while costing ~45 s.
    if (now_.received <= painted.received) continue;
    stalled.push(udid);
  }
  return stalled;
}

/// The delay required before attempt number `attempts` (0-based) for one udid.
///
/// Doubling, capped. Kept separate from the decision so the schedule can be asserted
/// directly rather than inferred from timing.
export function viewRecoveryDelayMs(
  attempts: number,
  baseMs: number = PAINT_RECOVERY_COOLDOWN_MS,
  maxMs: number = PAINT_RECOVERY_MAX_MS,
): number {
  if (attempts <= 0) return 0;
  const doubled = baseMs * 2 ** (attempts - 1);
  return Math.min(doubled, maxMs);
}

/// Whether enough time has passed to try recovering this udid again.
///
/// `attempts` is reset only by a frame actually being drawn, not by a restart succeeding:
/// the restart succeeding is exactly what kept happening while nothing painted.
export function shouldAttemptViewRecovery(
  udid: string,
  now: number,
  recoveredAt: Map<string, number>,
  attemptCounts: Map<string, number> = new Map(),
): boolean {
  const last = recoveredAt.get(udid);
  if (last === undefined) return true;
  return now - last >= viewRecoveryDelayMs(attemptCounts.get(udid) ?? 1);
}

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
  worker.onmessage = (event: MessageEvent<{ type: string; udid?: string; width?: number; height?: number; generation?: number; requestId?: number; bytes?: Uint8Array | null; frames?: number; received?: number; codecs?: string[] }>) => {
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
    if (message.type === "paintBeat" && message.udid) {
      const beat: ViewBeat = {
        at: Date.now(),
        received: message.received ?? 0,
        frames: message.frames ?? 0,
      };
      const previous = lastPaintBeat.get(message.udid);
      latestBeat.set(message.udid, beat);
      if (previous === undefined || beat.frames > previous.frames) {
        // A frame was genuinely drawn since the last check. That, and only that, clears the
        // backoff -- a restart that "succeeded" and still painted nothing must not buy
        // itself another fast retry.
        lastPaintBeat.set(message.udid, beat);
        recoveryAttempts.delete(message.udid);
        lastRecoveryAt.delete(message.udid);
        if (decodeFailed.delete(message.udid)) emit(message.udid);
      }
      return;
    }
    if (message.type === "decodeUnsupported" && message.udid) {
      // Was posted and dropped on the floor before this. A canvas that stays on its last
      // frame with nothing logged is indistinguishable from a phone that stopped changing,
      // which is how an undecodable stream survived 8 minutes unnoticed.
      console.error("view decode unsupported", message.udid, message.codecs);
      decodeFailed.add(message.udid);
      live.delete(message.udid);
      emit(message.udid);
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

function startStallWatch() {
  // Test mode stays timer-free so Vitest does not leak an interval; the policy is covered
  // through `collectStalledViews` / `shouldAttemptViewRecovery` instead.
  if (import.meta.env.MODE === "test") return;
  if (stallTimer != null) return;
  stallTimer = setInterval(() => {
    const now = Date.now();
    for (const udid of collectStalledViews(now, live, lastPaintBeat, latestBeat)) {
      live.delete(udid);
      emit(udid);
      if (!shouldAttemptViewRecovery(udid, now, lastRecoveryAt, recoveryAttempts)) continue;
      const attempt = (recoveryAttempts.get(udid) ?? 0) + 1;
      recoveryAttempts.set(udid, attempt);
      lastRecoveryAt.set(udid, now);
      const painted = lastPaintBeat.get(udid);
      const current = latestBeat.get(udid);
      console.warn(
        `view received ${(current?.received ?? 0) - (painted?.received ?? 0)} packets and drew ` +
          `nothing for ${PAINT_STALL_MS}ms; restarting ${udid} (attempt ${attempt}, next retry ` +
          `in ${Math.round(viewRecoveryDelayMs(attempt) / 1000)}s)`,
      );
      // viewEnsure restarts the producer at whatever preset the operator last asked for.
      void viewEnsure(udid).catch(() => {
        // The device may be unplugged, which is one of the reasons frames stop.
      });
    }
  }, STALL_TICK_MS);
}

/** Open the loopback view WebSocket and keep one worker for every canvas. */
export function startViewClient() {
  if (started) return;
  started = true;
  ensureWorker();
  startStallWatch();
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
