import { describe, expect, it } from "vitest";
import {
  annexBHasSps,
  annexBIsSyncSample,
  codecCandidatesFromAnnexB,
  codecFromAnnexB,
  decodeViewEnvelope,
  encodeViewEnvelope,
  shouldDecodeH264Sample,
  VIEW_KIND_H264,
  VIEW_MAGIC,
} from "./viewProtocol";

describe("viewProtocol", () => {
  it("round-trips the RVU1 header the Rust hub writes", () => {
    const payload = new Uint8Array([0, 0, 0, 1, 0x67, 0x42, 0xe0, 0x1e]);
    const encoded = encodeViewEnvelope({
      kind: "h264",
      key: true,
      generation: 3,
      width: 176,
      height: 392,
      udid: "ce06",
      payload,
    });
    expect(encoded[0]).toBe((VIEW_MAGIC >>> 24) & 0xff);
    expect(encoded[4]).toBe(VIEW_KIND_H264);
    const decoded = decodeViewEnvelope(encoded);
    expect(decoded).toEqual({
      kind: "h264",
      key: true,
      generation: 3,
      width: 176,
      height: 392,
      udid: "ce06",
      payload,
    });
  });

  it("reads a codec string from an Annex-B SPS", () => {
    const annexB = new Uint8Array([0, 0, 0, 1, 0x67, 0x64, 0x00, 0x1e, 0xaa, 0, 0, 0, 1, 0x65, 0x88]);
    expect(codecFromAnnexB(annexB)).toBe("avc1.64001E");
  });

  it("falls back from Note 8 Baseline 1.3 to a Constrained Baseline hint", () => {
    const note8 = new Uint8Array([0, 0, 0, 1, 0x67, 0x42, 0x00, 0x0d, 0xda]);
    expect(codecFromAnnexB(note8)).toBe("avc1.42000D");
    expect(codecCandidatesFromAnnexB(note8)[0]).toBe("avc1.42000D");
    expect(codecCandidatesFromAnnexB(note8)).toContain("avc1.42E01E");
  });

  it("rejects a buffer that is not RVU1", () => {
    expect(decodeViewEnvelope(new Uint8Array([1, 2, 3, 4]))).toBeNull();
  });

  it("treats an IDR without the key flag as a sync sample", () => {
    const idr = new Uint8Array([0, 0, 0, 1, 0x65, 0x88, 0, 0, 0, 1, 0x67, 0x42, 0xe0, 0x1e]);
    expect(annexBIsSyncSample(idr, false)).toBe(true);
    expect(annexBIsSyncSample(new Uint8Array([0, 0, 0, 1, 0x61, 0x00]), false)).toBe(false);
  });

  it("does not freeze a live decoder waiting for the next keyframe", () => {
    expect(shouldDecodeH264Sample(false, 0, false)).toBe(false);
    expect(shouldDecodeH264Sample(false, 0, true)).toBe(true);
    expect(shouldDecodeH264Sample(true, 0, false)).toBe(true);
    expect(shouldDecodeH264Sample(true, 2, false)).toBe(true);
    expect(shouldDecodeH264Sample(true, 3, false)).toBe(false);
    expect(shouldDecodeH264Sample(true, 3, true)).toBe(false);
  });
});

describe("the sample gate refuses keyframes too, which is why callers need an escape", () => {
  it("refuses a sync sample once a decoder exists and its queue is over the cap", () => {
    // This is the trap that produced a black overlay nothing could report: the gate is
    // `decodeQueueSize <= 2` for any existing decoder, with no exception for a keyframe. A
    // decoder that stops producing output keeps its queue above the cap forever, so every
    // later sample is refused -- including the keyframes that are the only thing that could
    // rebuild it. Packets keep arriving, nothing paints, no error is raised, and the codec
    // ladder is never reached so decodeUnsupported cannot fire either.
    //
    // The gate itself is correct: it bounds how far a live decoder may run behind. What was
    // missing is a caller that gives up on a decoder that never drains, which
    // viewDecode.worker.ts now does after MAX_QUEUE_REFUSALS. If this assertion ever starts
    // failing because the gate learned to let keyframes through, that escape can go.
    expect(shouldDecodeH264Sample(true, 3, true)).toBe(false);
    expect(shouldDecodeH264Sample(true, 99, true)).toBe(false);
  });

  it("still lets a keyframe build the first decoder, and still refuses a delta", () => {
    expect(shouldDecodeH264Sample(false, 0, true)).toBe(true);
    expect(shouldDecodeH264Sample(false, 0, false)).toBe(false);
  });

  it("keeps feeding a decoder that is keeping up", () => {
    expect(shouldDecodeH264Sample(true, 0, false)).toBe(true);
    expect(shouldDecodeH264Sample(true, 2, false)).toBe(true);
  });
});

describe("codecFromAnnexB fabricates a default, so callers must check for an SPS first", () => {
  const annexB = (...nals: number[][]) => {
    const out: number[] = [];
    for (const nal of nals) out.push(0, 0, 0, 1, ...nal);
    return new Uint8Array(out);
  };
  // NAL header byte: type is the low 5 bits. 0x67 = SPS(7), 0x65 = IDR(5), 0x41 = non-IDR(1).
  const sps = [0x67, 0x42, 0x00, 0x15, 0xaa];
  const idr = [0x65, 0x88, 0x84];
  const delta = [0x41, 0x9a, 0x02];

  it("reports no SPS for an IDR that does not carry one", () => {
    // The measured case. scrcpy sends config NALs separately, so a keyframe frequently has no
    // SPS -- and annexBIsSyncSample still calls it a sync sample, which is correct and is
    // exactly why "is it a keyframe" cannot stand in for "does it say which codec this is".
    expect(annexBHasSps(annexB(idr))).toBe(false);
    expect(annexBIsSyncSample(annexB(idr), false)).toBe(true);
  });

  it("reports an SPS when one is present", () => {
    expect(annexBHasSps(annexB(sps, idr))).toBe(true);
  });

  it("returns the hard-coded default for a blob with no SPS", () => {
    // Not a bug in this function -- a sensible last resort for building a FIRST decoder. The
    // bug was trusting it while a decoder already existed: on a real Galaxy S8 stream the
    // live codec was avc1.420015 and this fabrication is avc1.42E01E, so the comparison failed
    // on every SPS-less keyframe and rebuilt the decoder against the wrong string.
    expect(codecFromAnnexB(annexB(idr))).toBe("avc1.42E01E");
    expect(codecFromAnnexB(annexB(delta))).toBe("avc1.42E01E");
  });

  it("derives the real codec when the SPS is there", () => {
    expect(codecFromAnnexB(annexB(sps, idr))).toBe("avc1.420015");
  });
});
