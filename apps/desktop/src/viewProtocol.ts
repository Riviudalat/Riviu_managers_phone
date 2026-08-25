/**
 * Binary envelope the Rust ViewHub writes. Must stay byte-identical — and now is:
 * `the_worker_slices_the_envelope_this_file_writes` in `view_hub.rs` reads the five
 * constants below out of this file and compares them to the ones the encoder uses.
 */

export const VIEW_MAGIC = 0x5256_5531;
export const VIEW_KIND_H264 = 1;
export const VIEW_KIND_JPEG = 2;
export const VIEW_FLAG_KEY = 1;
export const VIEW_HEADER_BYTES = 24;

export type ViewKind = "h264" | "jpeg";

export interface ViewEnvelope {
  kind: ViewKind;
  key: boolean;
  generation: number;
  width: number;
  height: number;
  udid: string;
  payload: Uint8Array;
}

function readU32(bytes: Uint8Array, offset: number): number {
  return (
    ((bytes[offset] ?? 0) << 24) |
    ((bytes[offset + 1] ?? 0) << 16) |
    ((bytes[offset + 2] ?? 0) << 8) |
    (bytes[offset + 3] ?? 0)
  ) >>> 0;
}

function readU16(bytes: Uint8Array, offset: number): number {
  return ((bytes[offset] ?? 0) << 8) | (bytes[offset + 1] ?? 0);
}

function readU64(bytes: Uint8Array, offset: number): number {
  // Generations stay well below 2^53 for a desktop process.
  const hi = readU32(bytes, offset);
  const lo = readU32(bytes, offset + 4);
  return hi * 0x1_0000_0000 + lo;
}

export function decodeViewEnvelope(buffer: ArrayBuffer | Uint8Array): ViewEnvelope | null {
  const bytes = buffer instanceof Uint8Array ? buffer : new Uint8Array(buffer);
  if (bytes.length < VIEW_HEADER_BYTES) return null;
  if (readU32(bytes, 0) !== VIEW_MAGIC) return null;
  const kindByte = bytes[4];
  const kind: ViewKind | null = kindByte === VIEW_KIND_H264 ? "h264" : kindByte === VIEW_KIND_JPEG ? "jpeg" : null;
  if (!kind) return null;
  const flags = bytes[5] ?? 0;
  const generation = readU64(bytes, 6);
  const width = readU16(bytes, 14);
  const height = readU16(bytes, 16);
  const udidLen = readU16(bytes, 18);
  const payloadLen = readU32(bytes, 20);
  const start = VIEW_HEADER_BYTES;
  const udidEnd = start + udidLen;
  const payloadEnd = udidEnd + payloadLen;
  if (bytes.length < payloadEnd) return null;
  const udid = new TextDecoder().decode(bytes.subarray(start, udidEnd));
  return {
    kind,
    key: (flags & VIEW_FLAG_KEY) !== 0,
    generation,
    width,
    height,
    udid,
    payload: bytes.subarray(udidEnd, payloadEnd),
  };
}

export function encodeViewEnvelope(packet: ViewEnvelope): Uint8Array {
  const udid = new TextEncoder().encode(packet.udid);
  const out = new Uint8Array(VIEW_HEADER_BYTES + udid.length + packet.payload.length);
  out[0] = (VIEW_MAGIC >>> 24) & 0xff;
  out[1] = (VIEW_MAGIC >>> 16) & 0xff;
  out[2] = (VIEW_MAGIC >>> 8) & 0xff;
  out[3] = VIEW_MAGIC & 0xff;
  out[4] = packet.kind === "h264" ? VIEW_KIND_H264 : VIEW_KIND_JPEG;
  out[5] = packet.key ? VIEW_FLAG_KEY : 0;
  const gen = packet.generation;
  const hi = Math.floor(gen / 0x1_0000_0000);
  const lo = gen >>> 0;
  out[6] = (hi >>> 24) & 0xff;
  out[7] = (hi >>> 16) & 0xff;
  out[8] = (hi >>> 8) & 0xff;
  out[9] = hi & 0xff;
  out[10] = (lo >>> 24) & 0xff;
  out[11] = (lo >>> 16) & 0xff;
  out[12] = (lo >>> 8) & 0xff;
  out[13] = lo & 0xff;
  out[14] = (packet.width >>> 8) & 0xff;
  out[15] = packet.width & 0xff;
  out[16] = (packet.height >>> 8) & 0xff;
  out[17] = packet.height & 0xff;
  out[18] = (udid.length >>> 8) & 0xff;
  out[19] = udid.length & 0xff;
  const len = packet.payload.length;
  out[20] = (len >>> 24) & 0xff;
  out[21] = (len >>> 16) & 0xff;
  out[22] = (len >>> 8) & 0xff;
  out[23] = len & 0xff;
  out.set(udid, VIEW_HEADER_BYTES);
  out.set(packet.payload, VIEW_HEADER_BYTES + udid.length);
  return out;
}

export function annexBHasNal(bytes: Uint8Array, nalType: number): boolean {
  let i = 0;
  while (i + 4 < bytes.length) {
    let start = -1;
    if (bytes[i] === 0 && bytes[i + 1] === 0 && bytes[i + 2] === 0 && bytes[i + 3] === 1) {
      start = i + 4;
    } else if (bytes[i] === 0 && bytes[i + 1] === 0 && bytes[i + 2] === 1) {
      start = i + 3;
    }
    if (start < 0) {
      i += 1;
      continue;
    }
    if (((bytes[start] ?? 0) & 0x1f) === nalType) return true;
    i = start;
  }
  return false;
}

export function annexBIsSyncSample(bytes: Uint8Array, flaggedKey: boolean): boolean {
  return flaggedKey || annexBHasNal(bytes, 5) || annexBHasNal(bytes, 7);
}

/**
 * A live decoder must keep eating deltas. Dropping them until the next IDR
 * freezes the canvas for the whole `i-frame-interval` (1–2 s) — that is the
 * slideshow the operator sees after the encoder is already at 30 fps.
 * The worker pump already keeps only the latest packet; this only bounds
 * how far the decoder may run behind.
 */
export function shouldDecodeH264Sample(
  hasDecoder: boolean,
  decodeQueueSize: number,
  isSync: boolean,
): boolean {
  if (!hasDecoder) return isSync;
  return decodeQueueSize <= 2;
}

/// Whether this blob carries an SPS (NAL 7), i.e. whether it can say anything at all about
/// which codec the stream is.
///
/// Callers MUST check this before trusting [`codecFromAnnexB`], which returns a hard-coded
/// `avc1.42E01E` when no SPS is present. That default is a reasonable last resort for a first
/// decoder, and a trap for anything comparing against a codec already in use: scrcpy sends
/// config NALs separately and an IDR frequently arrives WITHOUT an SPS, so a sync sample is
/// not evidence of an SPS. Measured on a 20-device Galaxy S8 fleet -- the real streams were
/// `avc1.420015` (level 2.1), every SPS-less keyframe produced the fabricated
/// `avc1.42E01E` list, the mismatch tore the decoder down and rebuilt it against a codec
/// string the stream was not, and output stopped dead at ~50 frames per device.
export function annexBHasSps(bytes: Uint8Array): boolean {
  return annexBHasNal(bytes, 7);
}

/** Build `avc1.PPCCLL` from the first SPS NAL in an Annex-B blob. */
export function codecFromAnnexB(bytes: Uint8Array): string {
  const sps = findNal(bytes, 7);
  if (!sps || sps.length < 4) return "avc1.42E01E";
  const hex = (value: number) => value.toString(16).padStart(2, "0").toUpperCase();
  return `avc1.${hex(sps[1] ?? 0x42)}${hex(sps[2] ?? 0xe0)}${hex(sps[3] ?? 0x1e)}`;
}

/**
 * WebView2 `isConfigSupported` rejects some real encoder levels
 * (`avc1.42000D` = Baseline 1.3 on Note 8 152×320). The chunk still
 * carries the SPS; try a widely-accepted Constrained Baseline hint.
 */
export function codecCandidatesFromAnnexB(bytes: Uint8Array): string[] {
  const parsed = codecFromAnnexB(bytes);
  const fallbacks = ["avc1.42E01E", "avc1.42001E", "avc1.4D401E"];
  return [parsed, ...fallbacks.filter((codec) => codec !== parsed)];
}

function findNal(bytes: Uint8Array, type: number): Uint8Array | null {
  let i = 0;
  while (i + 4 < bytes.length) {
    let start = -1;
    let header = 3;
    if (bytes[i] === 0 && bytes[i + 1] === 0 && bytes[i + 2] === 1) {
      start = i + 3;
      header = 3;
    } else if (bytes[i] === 0 && bytes[i + 1] === 0 && bytes[i + 2] === 0 && bytes[i + 3] === 1) {
      start = i + 4;
      header = 4;
    }
    if (start < 0) {
      i += 1;
      continue;
    }
    let end = bytes.length;
    for (let j = start; j + 3 < bytes.length; j += 1) {
      if (bytes[j] === 0 && bytes[j + 1] === 0 && (bytes[j + 2] === 1 || (bytes[j + 2] === 0 && bytes[j + 3] === 1))) {
        end = j;
        break;
      }
    }
    const nal = bytes.subarray(start, end);
    if (((nal[0] ?? 0) & 0x1f) === type) return nal;
    i = start + (header === 4 ? 0 : 0);
    i = end;
  }
  return null;
}
