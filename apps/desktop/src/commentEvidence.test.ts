import { describe, expect, it } from "vitest";

import { evidenceLabel } from "./commentEvidence";
import type { NurtureApiTestResult } from "./types";

function result(over: Partial<NurtureApiTestResult>): NurtureApiTestResult {
  return {
    udid: "ce0717171c2a64d50d",
    comment: "hoa dep qua",
    caption: null,
    contextConfidence: 80,
    relevance: 80,
    evidenceSupport: 80,
    frameSha256: "abc",
    model: "deepseek-v4-flash-vision-exp",
    baseUrlHost: "api.deepseek.com",
    evidenceMode: "vision",
    distinctFrames: 3,
    promptTokens: 475,
    completionTokens: 135,
    ...over,
  };
}

describe("what the model actually looked at", () => {
  it("says one frame, and says why, when the card was still", () => {
    // The line this replaces read "3-frame vision" here — three thumbnails of one
    // byte-identical picture, described to the operator as three pieces of evidence.
    expect(evidenceLabel(result({ distinctFrames: 1 }))).toBe(
      "1 khung — bài tĩnh, không có chuyển động",
    );
  });

  it("counts the frames that really differed", () => {
    expect(evidenceLabel(result({ distinctFrames: 3 }))).toBe("3 khung khác nhau");
    expect(evidenceLabel(result({ distinctFrames: 2 }))).toBe("2 khung khác nhau");
  });

  it("never reports a frame count for the caption-only path", () => {
    // That path sends no picture, so `distinctFrames` is 0 and a count would be a lie either
    // way round — including the count the backend happens to send.
    expect(evidenceLabel(result({ evidenceMode: "ocr-caption", distinctFrames: 0 }))).toBe(
      "OCR caption + text",
    );
    expect(evidenceLabel(result({ evidenceMode: "ocr-caption", distinctFrames: 3 }))).toBe(
      "OCR caption + text",
    );
  });
});
