/** Minimal WebCodecs types for the view worker. The DOM lib may not include them. */

interface EncodedVideoChunkInit {
  type: "key" | "delta";
  timestamp: number;
  data: BufferSource;
}

declare class EncodedVideoChunk {
  constructor(init: EncodedVideoChunkInit);
}

interface VideoDecoderConfig {
  codec: string;
  optimizeForLatency?: boolean;
  hardwareAcceleration?: "no-preference" | "prefer-hardware" | "prefer-software";
}

interface VideoDecoderInit {
  output: (frame: VideoFrame) => void;
  error: (error: Error) => void;
}

declare class VideoDecoder {
  constructor(init: VideoDecoderInit);
  state: "unconfigured" | "configured" | "closed";
  decodeQueueSize: number;
  configure(config: VideoDecoderConfig): void;
  decode(chunk: EncodedVideoChunk): void;
  close(): void;
  static isConfigSupported(config: VideoDecoderConfig): Promise<{ supported: boolean }>;
}
