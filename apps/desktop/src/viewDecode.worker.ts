import {
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

type InMessage = AttachMessage | DetachMessage | PacketMessage | ExportMessage;

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
        error: () => {
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
    closeDecoder(slot);
    slot.generation = envelope.generation;
    slot.accelIndex = 0;
    slot.codecIndex = 0;
  }
  paintSize(slot, envelope.width, envelope.height);
  if (
    !shouldDecodeH264Sample(
      Boolean(slot.decoder),
      slot.decoder?.decodeQueueSize ?? 0,
      annexBIsSyncSample(envelope.payload, envelope.key),
    )
  ) {
    return;
  }
  const codecs = codecCandidatesFromAnnexB(envelope.payload);
  if (!slot.decoder || !codecs.includes(slot.codec ?? "")) {
    if (!annexBIsSyncSample(envelope.payload, envelope.key)) return;
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
    for (let candidate = slot.codecIndex; candidate < codecs.length; candidate += 1) {
      slot.codecIndex = candidate;
      slot.accelIndex = 0;
      slot.decoder = await configureDecoder(slot, codecs[candidate]);
      slot.codec = codecs[candidate];
      if (slot.decoder) break;
    }
    if (!slot.decoder) return;
  }
  const Chunk = (self as unknown as { EncodedVideoChunk: typeof EncodedVideoChunk }).EncodedVideoChunk;
  // 1 ms, not 1/15 s: some WebView2 decoders pace to the timestamp and a
  // 66 ms step turns a 3-frame queue into ~200 ms of extra glass delay.
  slot.timestamp += 1_000;
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
