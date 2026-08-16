import {
  annexBHasSps,
  annexBIsSyncSample,
  codecCandidatesFromAnnexB,
  decodeViewEnvelope,
  shouldDecodeH264Sample,
  type ViewEnvelope,
} from "./viewProtocol";

const ACCELS: VideoDecoderConfig["hardwareAcceleration"][] = [
  "prefer-hardware",
  "prefer-software",
  "no-preference",
];

interface AttachMessage {
  type: "attach";
  udid: string;
  surfaceId: string;
  canvas: OffscreenCanvas;
}

interface DetachMessage {
  type: "detach";
  udid: string;
  surfaceId: string;
}

interface PacketMessage {
  type: "packet";
  buffer: ArrayBuffer;
}

interface ExportMessage {
  type: "export";
  udid: string;
  requestId: number;
}

/// Turns the per-envelope counters on. Sent by the main thread rather than read from
/// `import.meta.env` in here: the worker's own DEV flag came back false under this build, so
/// the diagnostics silently printed nothing at exactly the moment they were needed.
interface DiagMessage {
  type: "diag";
  enabled: boolean;
}

type InMessage = AttachMessage | DetachMessage | PacketMessage | ExportMessage | DiagMessage;

interface Surface {
  id: string;
  canvas: OffscreenCanvas;
  ctx: OffscreenCanvasRenderingContext2D;
}

interface Slot {
  udid: string;
  surfaces: Surface[];
  decoder: VideoDecoder | null;
  generation: number;
  codec: string | null;
  timestamp: number;
  width: number;
  height: number;
  accelIndex: number;
  /// Which entry of `codecCandidatesFromAnnexB` is being tried.
  ///
  /// Exists because the async failure path had no way to advance the candidate list: it
  /// only moved `accelIndex`, and when that ran out it returned, so the fallback strings
  /// written for a decoder that rejects a valid config could never be reached.
  codecIndex: number;
  lastNotifiedW: number;
  lastNotifiedH: number;
  lastNotifiedGen: number;
  /// Frames actually drawn, and when the last heartbeat for them went out.
  ///
  /// `painted` cannot serve as a liveness signal: `notifyPainted` returns early unless the
  /// size or generation changed, so a stream that decodes steadily posts it once and never
  /// again. Measured consequence -- a producer whose frames stopped decoding held a stale
  /// canvas for 8 minutes while the Rust watchdog, which counts bytes arriving rather than
  /// frames drawn, stayed silent throughout.
  framesPainted: number;
  lastBeatAt: number;
  lastBeatFrames: number;
  /// Consecutive samples refused because the decoder's queue was not draining.
  ///
  /// `shouldDecodeH264Sample` returns `decodeQueueSize <= 2` once a decoder exists, and
  /// nothing in the old code could ever get out of that: a decoder that stops producing
  /// output keeps its queue above the cap, so every subsequent sample is refused --
  /// keyframes included, which are the only thing that could have rebuilt it. Packets keep
  /// arriving, nothing paints, no error is raised, and the codec ladder is never reached so
  /// `decodeUnsupported` cannot fire either. That is exactly the black overlay that survived
  /// two rounds of diagnosis.
  queueRefusals: number;
}

/// Consecutive queue refusals after which the decoder is rebuilt.
///
/// 48 is ~2 s at 24 fps -- comfortably longer than a decoder briefly running behind (which
/// is what the cap is for) and short enough that the operator sees a blip rather than a dead
/// canvas. Rebuilding is cheap: close, wait for the next keyframe, which scrcpy emits every
/// `i-frame-interval` (1 s here).
const MAX_QUEUE_REFUSALS = 48;

/// How often the worker reports what it received versus what it drew.
const PAINT_BEAT_MS = 1000;

/// Envelopes accepted off the socket per udid, painted frames aside.
///
/// Load-bearing for the stall rule, and the reason the first version of that rule was wrong:
/// scrcpy only encodes when the screen changes, so a phone parked on a static lock screen
/// legitimately paints nothing for minutes. "No frames drawn" therefore cannot mean broken.
/// The signal that does mean broken is packets arriving that produce no paint, which needs
/// both counts side by side.
const received = new Map<string, number>();

/// Per-envelope diagnostics, dev builds only.
///
/// Exists because the same question has now been unanswerable twice from code alone: an
/// overlay canvas stays black while packets demonstrably arrive, `decodeUnsupported` never
/// fires, and the Rust watchdog counts bytes rather than frames so it sees nothing wrong.
/// The three candidate mechanisms -- a keyframe that never arrived, a decoder that was fed
/// and produced no output, and a sample gate that refuses everything -- are indistinguishable
/// without counting each one separately.
let DIAG = false;

interface Diag {
  received: number;
  keys: number;
  fed: number;
  output: number;
  refusedNoDecoder: number;
  refusedQueue: number;
  refusedNotSync: number;
  closes: number;
  rebuilds: number;
  genChanges: number;
  lastCodec: string;
  lastCandidates: string;
  lastGeneration: number;
  lastReportAt: number;
}

const diag = new Map<string, Diag>();

function diagFor(udid: string): Diag {
  let entry = diag.get(udid);
  if (!entry) {
    entry = {
      received: 0,
      keys: 0,
      fed: 0,
      output: 0,
      refusedNoDecoder: 0,
      refusedQueue: 0,
      refusedNotSync: 0,
      closes: 0,
      rebuilds: 0,
      genChanges: 0,
      lastCodec: "",
      lastCandidates: "",
      lastGeneration: -1,
      lastReportAt: 0,
    };
    diag.set(udid, entry);
  }
  return entry;
}

/// Print at most once a second per device, and only when something moved.
function diagReport(udid: string, note: string) {
  if (!DIAG) return;
  const d = diagFor(udid);
  const now = performance.now();
  if (now - d.lastReportAt < 1000) return;
  d.lastReportAt = now;
  // console.warn, not console.info: vite forwards only warn/error from a client to the
  // terminal, so an info line here is visible in devtools and nowhere else -- which for a
  // diagnostic added to answer a question from the logs is the same as not existing.
  console.warn(
    `[viewdiag] ${udid} gen=${d.lastGeneration} recv=${d.received} keys=${d.keys} ` +
      `fed=${d.fed} out=${d.output} closes=${d.closes} ` +
      `refused(nodec=${d.refusedNoDecoder} queue=${d.refusedQueue} notsync=${d.refusedNotSync}) ` +
      `${note}`,
  );
}

const slots = new Map<string, Slot>();
const pending = new Map<string, ViewEnvelope>();
const queued = new Map<string, ViewEnvelope>();
const decoding = new Set<string>();

function jpegCopy(bytes: Uint8Array): Uint8Array {
  const copy = new Uint8Array(bytes.byteLength);
  copy.set(bytes);
  return copy;
}

function paintSize(slot: Slot, width: number, height: number) {
  slot.width = width || slot.width;
  slot.height = height || slot.height;
  if (slot.width <= 0 || slot.height <= 0) return;
  for (const surface of slot.surfaces) {
    if (surface.canvas.width !== slot.width || surface.canvas.height !== slot.height) {
      surface.canvas.width = slot.width;
      surface.canvas.height = slot.height;
    }
  }
}

function drawFrame(slot: Slot, source: CanvasImageSource) {
  for (const surface of slot.surfaces) {
    surface.ctx.drawImage(source, 0, 0, surface.canvas.width, surface.canvas.height);
  }
}

function closeDecoder(slot: Slot) {
  if (DIAG) diagFor(slot.udid).closes += 1;
  if (slot.decoder && slot.decoder.state !== "closed") {
    try {
      slot.decoder.close();
    } catch {
      // already closed
    }
  }
  slot.decoder = null;
  slot.codec = null;
}

function beatPainted(slot: Slot) {
  slot.framesPainted += 1;
  const now = performance.now();
  if (now - slot.lastBeatAt < PAINT_BEAT_MS) return;
  slot.lastBeatAt = now;
  slot.lastBeatFrames = slot.framesPainted;
  emitBeat(slot.udid, slot.generation, slot.framesPainted);
}

/// Report received-vs-painted for one udid. Sent from the paint path AND from the arrival
/// path, because the case worth catching is arrivals climbing while paints do not, and the
/// paint path by definition is not running then.
function emitBeat(udid: string, generation: number, frames: number) {
  const d = DIAG ? diagFor(udid) : undefined;
  postMessage({
    type: "paintBeat",
    udid,
    generation,
    frames,
    received: received.get(udid) ?? 0,
    // Carried on the beat rather than logged from in here. Vite forwards the PAGE's console
    // to the terminal, not a Web Worker's, so every diagnostic printed from this file went
    // to devtools and nowhere else -- which is why the counters read zero at exactly the
    // moment they were supposed to answer the question.
    diag: d
      ? {
          fed: d.fed,
          output: d.output,
          closes: d.closes,
          noDecoder: d.refusedNoDecoder,
          queue: d.refusedQueue,
          notSync: d.refusedNotSync,
          keys: d.keys,
          rebuilds: d.rebuilds,
          genChanges: d.genChanges,
          lastCodec: d.lastCodec,
          lastCandidates: d.lastCandidates,
        }
      : undefined,
  });
}

const lastArrivalBeatAt = new Map<string, number>();

function beatArrival(udid: string) {
  received.set(udid, (received.get(udid) ?? 0) + 1);
  if (DIAG) diagFor(udid).received += 1;
  const now = performance.now();
  const last = lastArrivalBeatAt.get(udid) ?? 0;
  if (now - last < PAINT_BEAT_MS) return;
  lastArrivalBeatAt.set(udid, now);
  const slot = slots.get(udid);
  emitBeat(udid, slot?.generation ?? 0, slot?.framesPainted ?? 0);
}

function notifyPainted(udid: string, slot: Slot) {
  if (
    slot.lastNotifiedW === slot.width &&
    slot.lastNotifiedH === slot.height &&
    slot.lastNotifiedGen === slot.generation
  ) {
    return;
  }
  slot.lastNotifiedW = slot.width;
  slot.lastNotifiedH = slot.height;
  slot.lastNotifiedGen = slot.generation;
  postMessage({
    type: "painted",
    udid,
    width: slot.width,
    height: slot.height,
    generation: slot.generation,
  });
}

async function configureDecoder(slot: Slot, codec: string): Promise<VideoDecoder | null> {
  const Ctor = (self as unknown as { VideoDecoder?: typeof VideoDecoder }).VideoDecoder;
  if (!Ctor) return null;

  const output = (frame: VideoFrame) => {
    try {
      const width = frame.displayWidth || frame.codedWidth;
      const height = frame.displayHeight || frame.codedHeight;
      paintSize(slot, width, height);
      drawFrame(slot, frame);
      notifyPainted(slot.udid, slot);
      beatPainted(slot);
      if (DIAG) diagFor(slot.udid).output += 1;
    } finally {
      frame.close();
    }
  };

  const tryConfig = async (hardwareAcceleration: VideoDecoderConfig["hardwareAcceleration"]) => {
    const config: VideoDecoderConfig = {
      codec,
      optimizeForLatency: true,
      hardwareAcceleration,
    };
    if (slot.width > 0 && slot.height > 0) {
      config.codedWidth = slot.width;
      config.codedHeight = slot.height;
    }
    // Note 8 Baseline 1.3 (`avc1.42000D`) can be reported unsupported
    // and still decode if we configure anyway.
    try {
      const decoder = new Ctor({
        output,
        error: (cause: unknown) => {
          // Decoder errors were 100% invisible before this: the callback logged nothing, the
          // worker has no `onerror` wired, and the only channel that ever surfaced a decode
          // problem was `decodeUnsupported`, which this very path made unreachable.
          // postMessage, not console: vite forwards the page's console to the terminal but
          // NOT a worker's, so every decoder error logged from in here went to devtools and
          // nowhere else. That is why "decoder rejected" read zero in the logs while the
          // counters showed the decoder being closed and rebuilt four times per device.
          postMessage({
            type: "decoderError",
            udid: slot.udid,
            codec,
            accel: slot.accelIndex,
            candidate: slot.codecIndex,
            errorMessage: cause instanceof Error ? `${cause.name}: ${cause.message}` : String(cause),
          });
          closeDecoder(slot);
          const held = pending.get(slot.udid);
          if (!held) return;
          // Advance the ladder in BOTH dimensions. This used to move `accelIndex` only
          // and give up silently once it ran out, which meant the codec candidates
          // written for exactly this failure — a config the decoder accepts
          // syntactically and then rejects asynchronously — were unreachable. Three
          // accels later the slot had no decoder, nothing was reported, and every
          // surface on that udid stayed black while packets kept arriving.
          if (slot.accelIndex + 1 < ACCELS.length) {
            slot.accelIndex += 1;
          } else {
            slot.accelIndex = 0;
            slot.codecIndex += 1;
            // Clears the "already configured for this codec" check in `handleH264` so
            // the candidate loop runs again from the new cursor.
            slot.codec = null;
          }
          void handleH264(slot, held);
        },
      });
      decoder.configure(config);
      return decoder;
    } catch {
      return null;
    }
  };

  for (let i = slot.accelIndex; i < ACCELS.length; i += 1) {
    slot.accelIndex = i;
    const decoder = await tryConfig(ACCELS[i]);
    if (decoder) return decoder;
  }
  return null;
}

async function handleH264(slot: Slot, envelope: ViewEnvelope) {
  if (envelope.kind !== "h264") return;
  if (envelope.generation !== slot.generation) {
    if (DIAG) diagFor(slot.udid).genChanges += 1;
    closeDecoder(slot);
    slot.generation = envelope.generation;
    slot.accelIndex = 0;
    slot.codecIndex = 0;
  }
  paintSize(slot, envelope.width, envelope.height);
  const isSync = annexBIsSyncSample(envelope.payload, envelope.key);
  if (DIAG) {
    const d = diagFor(slot.udid);
    d.lastGeneration = slot.generation;
    if (isSync) d.keys += 1;
  }
  if (!shouldDecodeH264Sample(Boolean(slot.decoder), slot.decoder?.decodeQueueSize ?? 0, isSync)) {
    if (!slot.decoder) {
      // Waiting for a keyframe to build a decoder with. Normal and self-clearing.
      if (DIAG) {
        diagFor(slot.udid).refusedNoDecoder += 1;
        diagReport(slot.udid, "no decoder yet");
      }
      return;
    }
    // A decoder exists and its queue is over the cap. Brief is normal; permanent is the
    // trap, because the refusal also blocks the keyframes that would fix it.
    slot.queueRefusals += 1;
    if (DIAG) {
      diagFor(slot.udid).refusedQueue += 1;
      diagReport(slot.udid, `queue=${slot.decoder.decodeQueueSize} refusals=${slot.queueRefusals}`);
    }
    if (slot.queueRefusals < MAX_QUEUE_REFUSALS) return;
    // Break out. Closing sets slot.decoder to null, so the next sync sample rebuilds from
    // scratch instead of feeding a decoder that has stopped producing output.
    console.warn(
      `view decoder for ${slot.udid} stopped draining (${slot.queueRefusals} refusals); rebuilding`,
    );
    closeDecoder(slot);
    slot.queueRefusals = 0;
    slot.accelIndex = 0;
    slot.codecIndex = 0;
    if (!isSync) return;
  } else {
    slot.queueRefusals = 0;
  }
  // Only a sample that CARRIES an SPS can say anything about which codec is right. A delta
  // has none, so `codecFromAnnexB` falls back to the literal "avc1.42E01E" and the candidate
  // list it produces is fiction -- if `slot.codec` was derived from a real SPS, that fiction
  // never contains it, every delta was therefore sent down the rebuild path, and the
  // `!isSync` guard dropped it. Every P-frame discarded, and the decoder town down and
  // rebuilt once per keyframe.
  // Re-derive the codec ONLY from a packet that actually carries an SPS. A sync sample is not
  // enough: scrcpy sends config NALs separately, so an IDR often arrives without one, and
  // `codecFromAnnexB` answers that with a hard-coded `avc1.42E01E` rather than "I don't know".
  // Comparing a live `slot.codec` against that fabrication fails every time, which tore the
  // decoder down on each such keyframe and rebuilt it against a codec string the stream was
  // not. Measured across a 20-device fleet: `codec=avc1.420015` versus
  // `cands=avc1.42E01E,avc1.42001E,avc1.4D401E`, and output stopped at ~50 frames each.
  const canJudgeCodec = annexBHasSps(envelope.payload);
  if (!slot.decoder) {
    // Nothing to keep; fall through and build one.
  } else if (!canJudgeCodec) {
    // Live decoder and no evidence about the codec: feed it.
  }
  if (!slot.decoder || (canJudgeCodec && !codecCandidatesFromAnnexB(envelope.payload).includes(slot.codec ?? ""))) {
    const codecs = codecCandidatesFromAnnexB(envelope.payload);
    if (DIAG) {
      const d = diagFor(slot.udid);
      d.rebuilds += 1;
      d.lastCodec = slot.codec ?? "(none)";
      d.lastCandidates = codecs.join(",");
    }
    if (!isSync) return;
    closeDecoder(slot);
    if (slot.codecIndex >= codecs.length) {
      // Every candidate x every acceleration mode has been refused. Say so once per
      // generation: a black canvas with a silent worker is the state this whole ladder
      // exists to avoid, and the operator cannot tell it from a phone that stopped
      // sending.
      if (slot.lastNotifiedGen !== slot.generation) {
        slot.lastNotifiedGen = slot.generation;
        postMessage({
          type: "decodeUnsupported",
          udid: slot.udid,
          generation: slot.generation,
          codecs,
        });
      }
      return;
    }
    // `slot.accelIndex` must survive re-entry. The async `error` callback advances it and
    // then calls back into here; resetting it unconditionally destroyed that advance, so the
    // ladder retried the SAME codec at the SAME acceleration forever -- a hot spin, one
    // VideoDecoder construct+configure per rejected frame, nothing painted, and `codecIndex`
    // frozen at 0 because it only moves once `accelIndex` reaches the last mode. That is why
    // `decodeUnsupported` was provably unreachable while the canvas stayed black.
    const startCandidate = slot.codecIndex;
    for (let candidate = startCandidate; candidate < codecs.length; candidate += 1) {
      // A genuinely new candidate starts at hardware again; the one we were already on keeps
      // whatever mode the error path had moved it to.
      if (candidate !== startCandidate) slot.accelIndex = 0;
      slot.codecIndex = candidate;
      slot.decoder = await configureDecoder(slot, codecs[candidate]);
      slot.codec = codecs[candidate];
      if (slot.decoder) break;
    }
    if (!slot.decoder) {
      // Every candidate refused `configure()` outright. The loop leaves `codecIndex` at
      // `length - 1`, so the exhaustion test above could never fire and the next sync sample
      // would retry only the last fallback, forever and silently. Mark it truly exhausted so
      // the operator is told; a generation change resets this.
      slot.codecIndex = codecs.length;
      return;
    }
  }
  // Narrows for the compiler, and is a real guard: the branch above can decline to rebuild.
  if (!slot.decoder) return;
  const Chunk = (self as unknown as { EncodedVideoChunk: typeof EncodedVideoChunk }).EncodedVideoChunk;
  // 1 ms, not 1/15 s: some WebView2 decoders pace to the timestamp and a
  // 66 ms step turns a 3-frame queue into ~200 ms of extra glass delay.
  slot.timestamp += 1_000;
  if (DIAG) {
    diagFor(slot.udid).fed += 1;
    diagReport(slot.udid, `queue=${slot.decoder.decodeQueueSize}`);
  }
  slot.decoder.decode(
    new Chunk({
      type: envelope.key ? "key" : "delta",
      timestamp: slot.timestamp,
      data: jpegCopy(envelope.payload),
    }),
  );
}

async function handleJpeg(udid: string, slot: Slot, envelope: NonNullable<ReturnType<typeof decodeViewEnvelope>>) {
  const jpeg = new Uint8Array(envelope.payload.byteLength);
  jpeg.set(envelope.payload);
  const blob = new Blob([jpeg], { type: "image/jpeg" });
  const bitmap = await createImageBitmap(blob);
  try {
    paintSize(slot, envelope.width || bitmap.width, envelope.height || bitmap.height);
    drawFrame(slot, bitmap);
    notifyPainted(udid, slot);
    beatPainted(slot);
  } finally {
    bitmap.close();
  }
}

self.onmessage = (event: MessageEvent<InMessage>) => {
  const message = event.data;
  if (message.type === "attach") {
    const ctx = message.canvas.getContext("2d", { desynchronized: true, alpha: false });
    if (!ctx) return;
    const slot = slots.get(message.udid) ?? {
      udid: message.udid,
      surfaces: [],
      decoder: null,
      generation: 0,
      codec: null,
      timestamp: 0,
      width: 0,
      height: 0,
      accelIndex: 0,
      codecIndex: 0,
      framesPainted: 0,
      lastBeatAt: 0,
      lastBeatFrames: 0,
      queueRefusals: 0,
      lastNotifiedW: 0,
      lastNotifiedH: 0,
      lastNotifiedGen: -1,
    };
    slot.surfaces = slot.surfaces.filter((surface) => surface.id !== message.surfaceId);
    slot.surfaces.push({ id: message.surfaceId, canvas: message.canvas, ctx });
    if (slot.width > 0 && slot.height > 0) {
      message.canvas.width = slot.width;
      message.canvas.height = slot.height;
    }
    slots.set(message.udid, slot);
    const held = pending.get(message.udid);
    if (held) {
      if (held.kind === "h264") {
        pumpH264(slot, held);
      } else {
        void handleJpeg(message.udid, slot, held);
      }
    }
    return;
  }
  if (message.type === "diag") {
    DIAG = message.enabled === true;
    return;
  }
  if (message.type === "detach") {
    const slot = slots.get(message.udid);
    if (!slot) return;
    slot.surfaces = slot.surfaces.filter((surface) => surface.id !== message.surfaceId);
    if (slot.surfaces.length === 0) {
      closeDecoder(slot);
      slots.delete(message.udid);
    }
    return;
  }
  if (message.type === "export") {
    const slot = slots.get(message.udid);
    const requestId = message.requestId;
    const canvas = slot?.surfaces[0]?.canvas;
    if (!slot || !canvas || typeof canvas.convertToBlob !== "function") {
      postMessage({ type: "exportResult", requestId, bytes: null });
      return;
    }
    void canvas
      .convertToBlob({ type: "image/jpeg", quality: 0.85 })
      .then(async (blob) => {
        postMessage({
          type: "exportResult",
          requestId,
          bytes: new Uint8Array(await blob.arrayBuffer()),
        });
      })
      .catch(() => {
        postMessage({ type: "exportResult", requestId, bytes: null });
      });
    return;
  }
  if (message.type !== "packet") return;
  const envelope = decodeViewEnvelope(message.buffer);
  if (!envelope) return;
  beatArrival(envelope.udid);
  const previous = pending.get(envelope.udid);
  if (previous && previous.generation !== envelope.generation) {
    pending.delete(envelope.udid);
  }
  if (
    envelope.key ||
    (envelope.kind === "h264" && annexBIsSyncSample(envelope.payload, envelope.key))
  ) {
    pending.set(envelope.udid, envelope);
  }
  const slot = slots.get(envelope.udid);
  if (!slot) return;
  if (envelope.kind === "h264") {
    pumpH264(slot, envelope);
    return;
  }
  // H.264 owns this surface. A leftover minicap JPEG on the same UDID
  // would close the motion path and paint a still every preview tick.
  if (slot.decoder) return;
  void handleJpeg(envelope.udid, slot, envelope);
};

function pumpH264(slot: Slot, envelope: ViewEnvelope) {
  queued.set(slot.udid, envelope);
  if (decoding.has(slot.udid)) return;
  decoding.add(slot.udid);
  void (async () => {
    try {
      while (queued.has(slot.udid)) {
        const next = queued.get(slot.udid);
        queued.delete(slot.udid);
        if (next) await handleH264(slot, next);
      }
    } finally {
      decoding.delete(slot.udid);
    }
  })();
}
