import { useEffect, useSyncExternalStore } from "react";
import { viewEndpoint, viewReportPaint, type ViewPaintReport } from "./api";

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
/// The beat at which each udid last actually drew, and the newest beat of any kind.
const lastPaintBeat = new Map<string, ViewBeat>();
const latestBeat = new Map<string, ViewBeat>();

/// This module does not restart anything, and that is the design rather than a limitation.
///
/// It used to. Restarting on "packets arrive but nothing paints" is a positive feedback
/// loop, and it got worse with every scale it was measured at: 2 phones produced 33 producer
/// starts for 3 overlay open/close cycles, and 20 phones produced **291**. Each restart
/// costs adb work and CPU, which makes more devices miss their paint window, which triggers
/// more restarts. At fleet scale the recovery destroyed the thing it was recovering, so the
/// behaviour was left switched off behind a flag.
///
/// The flag is gone now because the split it guarded is gone. Deciding here was never the
/// right shape: this process cannot see whether a start is already in flight, cannot see
/// what another window is doing, loses every counter on reload, and cannot bound anything
/// fleet-wide. What it *can* see — and nothing else can — is whether a frame came out of the
/// decoder. So that is all it does: it reports the counters (`view_report_paint`) and drops
/// its own tile out of Live, which is honest and costs nothing. One decision, one backoff
/// and one fleet-wide ceiling now live in `view_watchdog.rs` on the Rust side.
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
/// Kept in step with `VIEW_PAINT_STALL` in `view_watchdog.rs`. Two copies because two
/// processes need it and there is no shared constant between them — but only one of them
/// decides anything with it. Here it draws the tile's "not live" state; there it decides
/// whether a producer is restarted. If they ever disagree the tile is briefly honest about
/// something the host has not acted on yet, which is the harmless direction.
export const PAINT_STALL_MS = 12000;
const STALL_TICK_MS = 2000;

let stallTimer: ReturnType<typeof setInterval> | null = null;

export function viewDecodeFailed(udid: string): boolean {
  return decodeFailed.has(udid);
}

/// One udid's most recent worker beat: how many envelopes arrived and how many frames were
/// drawn, as of `at`.
export interface ViewDiag {
  fed: number;
  output: number;
  closes: number;
  noDecoder: number;
  queue: number;
  notSync: number;
  keys: number;
  rebuilds: number;
  genChanges: number;
  lastCodec: string;
  lastCandidates: string;
}

export interface ViewBeat {
  at: number;
  /// Which producer these counters belong to.
  ///
  /// The worker has always sent it and this store used to throw it away. It is what lets
  /// the host tell evidence about the running producer from evidence about the one it just
  /// replaced — counters captured before a restart show arrivals far ahead of frames
  /// forever, and a host that acted on them would restart the moment each restart finished.
  generation: number;
  received: number;
  frames: number;
  diag?: ViewDiag;
}

/// The udids that look stalled: packets kept arriving and no frame was drawn for them.
///
/// This decides the tile's Live flag and nothing else — the restart decision it used to feed
/// now lives in Rust, where the fleet-wide ceiling is. Kept here because dropping a tile out
/// of Live the moment its decoder stops producing is honest, immediate and free, and because
/// this is the predicate that distinguishes a dead decoder from a screen that is not moving.
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

/// Everything the host's watchdog needs, for every device this window is tracking.
///
/// Healthy devices are included deliberately. A report that only ever named broken devices
/// would leave the host unable to tell "nothing is wrong" from "nobody is reporting", and
/// those two must lead to different behaviour: the second one has to fall back to the coarse
/// byte rule rather than trust a paint rule nobody is feeding.
///
/// Ages, not timestamps: the WebView's clock and the host's are different clocks.
export function collectPaintReports(
  now: number,
  latest: Map<string, ViewBeat>,
  lastPaint: Map<string, ViewBeat>,
): ViewPaintReport[] {
  const reports: ViewPaintReport[] = [];
  for (const [udid, beat] of latest) {
    const painted = lastPaint.get(udid);
    reports.push({
      udid,
      generation: beat.generation,
      received: beat.received,
      frames: beat.frames,
      // Never painted: report the age of the stream itself, and let `frames === 0` be what
      // tells the host this is a device starting up rather than one that stopped.
      sincePaintMs: Math.max(0, now - (painted?.at ?? beat.at)),
    });
  }
  return reports;
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
  // The worker's own `import.meta.env.DEV` came back false under this build, so its
  // diagnostics never printed. Read the flag here, where it demonstrably works, and tell the
  // worker.
  // Always on: the counters are only emitted on a beat that is already being sent, and
  // they are only printed when a device is reported stalled, so the cost is a few fields.
  worker.postMessage({ type: "diag", enabled: true });
  worker.onmessage = (event: MessageEvent<{ type: string; udid?: string; width?: number; height?: number; generation?: number; requestId?: number; bytes?: Uint8Array | null; frames?: number; received?: number; diag?: ViewDiag; codecs?: string[]; codec?: string; accel?: number; candidate?: number; errorMessage?: string }>) => {
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
        generation: message.generation ?? 0,
        received: message.received ?? 0,
        frames: message.frames ?? 0,
        diag: message.diag,
      };
      const previous = lastPaintBeat.get(message.udid);
      latestBeat.set(message.udid, beat);
      if (previous === undefined || beat.frames > previous.frames) {
        // A frame was genuinely drawn since the last check.
        lastPaintBeat.set(message.udid, beat);
        // Painting again is what makes a view live again. Only the `painted` message used to
        // do this, and that fires solely when the size or generation CHANGES -- so after a
        // stall marked a device not-live, a stream that recovered at the same resolution kept
        // "Dang cho stream..." printed over a picture that was visibly running.
        const recovered = decodeFailed.delete(message.udid);
        const wasDark = !live.has(message.udid);
        live.add(message.udid);
        if (recovered || wasDark) emit(message.udid);
      }
      return;
    }
    if (message.type === "decoderError" && message.udid) {
      console.warn(
        `view decoder error ${message.udid} codec=${message.codec} accel=${message.accel} ` +
          `candidate=${message.candidate}: ${message.errorMessage}`,
      );
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
  // through `collectStalledViews` / `collectPaintReports` instead.
  if (import.meta.env.MODE === "test") return;
  if (stallTimer != null) return;
  stallTimer = setInterval(() => {
    const now = Date.now();
    // Every device, not only the stalled ones. The host has to be able to tell a healthy
    // fleet from a window that has stopped reporting, and it can only do that if a healthy
    // report is a thing that arrives.
    if (latestBeat.size > 0) {
      void viewReportPaint(collectPaintReports(now, latestBeat, lastPaintBeat)).catch(() => {
        // The host is shutting down, or the command is not registered in a harness. Losing a
        // report costs the watchdog its fine rule for a few seconds, not its coarse one.
      });
    }
    for (const udid of collectStalledViews(now, live, lastPaintBeat, latestBeat)) {
      // Dropping the tile out of Live is this module's whole remaining job on a stall. It is
      // honest, it is instant, and it costs nothing to be wrong about for one tick.
      live.delete(udid);
      emit(udid);
      const painted = lastPaintBeat.get(udid);
      const current = latestBeat.get(udid);
      // vite forwards the PAGE's console, never a Web Worker's (AGENTS.md 9.66), so this is
      // the only line on which the worker's counters ever reach a terminal. It stays on the
      // main thread for that reason, and it prints where a reader is already looking: on the
      // line that says something is wrong.
      console.warn(
        `view received ${(current?.received ?? 0) - (painted?.received ?? 0)} packets and drew ` +
          `nothing for ${PAINT_STALL_MS}ms on ${udid}; reported to the host watchdog` +
          (current?.diag
            ? ` [fed=${current.diag.fed} out=${current.diag.output} keys=${current.diag.keys} ` +
              `closes=${current.diag.closes} refused(nodec=${current.diag.noDecoder} ` +
              `queue=${current.diag.queue} notsync=${current.diag.notSync}) ` +
              `rebuilds=${current.diag.rebuilds} genchg=${current.diag.genChanges} ` +
              `codec=${current.diag.lastCodec} cands=${current.diag.lastCandidates}]`
            : ""),
      );
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

/**
 * Several JPEGs of one device, spaced far enough apart to differ.
 *
 * The grounded-comment pipeline reads up to three frames, because a still and a scrolling
 * feed are different evidence. Devices on the H.264 view path publish nothing into the
 * host's JPEG hub, so this is where the host's copy of that evidence has to come from.
 *
 * A device this worker is not decoding returns `[]` rather than throwing — the caller then
 * has nothing to send, and the backend falls back to the hub, which is exactly right for a
 * device whose frames live there.
 */
export async function exportViewJpegBurst(
  udid: string,
  count = 3,
  gapMs = 250,
): Promise<Uint8Array[]> {
  const frames: Uint8Array[] = [];
  for (let index = 0; index < count; index += 1) {
    const bytes = await exportViewJpeg(udid);
    // A first frame that is absent means this device is not on the view path at all;
    // stop rather than spend the gaps discovering it twice more.
    if (!bytes) break;
    frames.push(bytes);
    if (index + 1 < count) await new Promise((resolve) => setTimeout(resolve, gapMs));
  }
  return frames;
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

/// Whether every codec candidate refused this device's stream.
///
/// `viewDecodeFailed` has existed since the decoder gave up silently, and until now nothing
/// read it — so an undecodable stream showed the same "still coming" state as one that was
/// merely slow, forever. Both the add and the recovery `emit`, so this re-renders correctly
/// in either direction.
export function useViewDecodeFailed(udid: string): boolean {
  return useSyncExternalStore(
    (onStoreChange) => subscribe(udid, onStoreChange),
    () => decodeFailed.has(udid),
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
