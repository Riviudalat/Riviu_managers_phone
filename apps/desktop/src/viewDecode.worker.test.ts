import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ViewEnvelope } from "./viewProtocol";
const fixtures = vi.hoisted(() => new Map<ArrayBuffer, ViewEnvelope>());
vi.mock("./viewProtocol", async importOriginal => ({
  ...await importOriginal<typeof import("./viewProtocol")>(),
  decodeViewEnvelope: (buffer: ArrayBuffer) => fixtures.get(buffer),
}));
let messages: ReturnType<typeof vi.fn>;
let workers: { onmessage?: (event: { data: unknown }) => void; VideoDecoder: typeof Decoder; EncodedVideoChunk: typeof Chunk };
class Chunk {
  data: { type: string; timestamp: number; data: Uint8Array };
  constructor(data: Chunk["data"]) { this.data = data; }
}
class Decoder {
  static all: Decoder[] = [];
  decodeQueueSize = 0;
  state = "configured";
  chunks: Chunk[] = [];
  private options: { output: (frame: unknown) => void };
  constructor(options: { output: (frame: unknown) => void }) { this.options = options; Decoder.all.push(this); }
  configure() {}
  close() { this.state = "closed"; }
  decode(chunk: Chunk) {
    this.chunks.push(chunk);
    this.options.output({ timestamp: chunk.data.timestamp, displayWidth: 288, displayHeight: 600, close: vi.fn() });
  }
}
const send = (data: unknown) => workers.onmessage?.({ data });
function attach(udid: string, surfaceId: string) {
  const drawImage = vi.fn();
  send({ type: "attach", udid, surfaceId, canvas: { getContext: () => ({ drawImage }) } });
  return drawImage;
}
async function packet(udid: string, nal: number, key = false) {
  const buffer = new ArrayBuffer(1);
  fixtures.set(buffer, { udid, kind: "h264", generation: 1, width: 288, height: 600, key,
    payload: new Uint8Array([...(key && nal === 0x65 ? [0, 0, 0, 1, 0x67, 0x42, 0, 0x1e] : []), 0, 0, 0, 1, nal, 0x42, 0, 0x1e, 0xaa]) } as ViewEnvelope);
  send({ type: "packet", buffer });
  await Promise.resolve(); await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
}
beforeEach(async () => {
  vi.resetModules(); vi.useFakeTimers(); fixtures.clear(); Decoder.all = [];
  messages = vi.fn(); workers = { VideoDecoder: Decoder, EncodedVideoChunk: Chunk };
  vi.stubGlobal("self", workers); vi.stubGlobal("postMessage", messages);
  await import("./viewDecode.worker");
});
afterEach(() => { vi.clearAllTimers(); vi.useRealTimers(); vi.unstubAllGlobals(); });
describe("actual view worker", () => {
  it("pauses the focused tile, shares its decoder and immediately restores the tile", async () => {
    const tile = attach("A", "tile");
    await packet("A", 0x65, true);
    expect(tile).toHaveBeenCalled();
    const decoder = Decoder.all[0];
    const focus = attach("A", "overlay"); tile.mockClear();
    await packet("A", 0x41);
    expect(tile).not.toHaveBeenCalled(); expect(focus).toHaveBeenCalled();
    expect(Decoder.all).toHaveLength(1); expect(decoder.state).toBe("configured");
    send({ type: "detach", udid: "A", surfaceId: "overlay" });
    expect(tile).toHaveBeenCalled(); expect(Decoder.all).toHaveLength(1);
  });
  it("paints the last background frame on the next tick even when motion stops", async () => {
    attach("A", "overlay"); const tile = attach("B", "tile");
    await packet("B", 0x65, true); tile.mockClear();
    await packet("B", 0x41); expect(tile).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(210);
    expect(tile).toHaveBeenCalledTimes(1);
  });
  it("recovers from overload only at an IDR, preserving configuration and limiting resync requests", async () => {
    attach("A", "overlay"); await packet("A", 0x67, true);
    Decoder.all[0].decodeQueueSize = 3;
    await packet("A", 0x41); expect(Decoder.all[0].state).toBe("closed");
    await packet("A", 0x41); await packet("A", 0x67, true);
    expect(Decoder.all).toHaveLength(1);
    expect(messages.mock.calls.filter(([event]) => event.type === "requestKeyframe")).toHaveLength(1);
    await packet("A", 0x65);
    expect(Decoder.all).toHaveLength(2);
    expect(Decoder.all[1].chunks.at(-1)?.data.type).toBe("key");
    expect(Decoder.all[1].chunks.at(-1)?.data.data.length).toBeGreaterThan(9);
  });
});
