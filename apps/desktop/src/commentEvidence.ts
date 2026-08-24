/**
 * What the model was actually shown, in one phrase.
 *
 * Deliberately a plain function over the two fields both shapes carry, rather than two
 * copies: the manual API test (`NurtureApiTestResult`) and a session's audit row
 * (`NurtureCommentAttempt`) describe the same picture, and a phrase that drifts between them
 * is a phrase an operator cannot compare.
 */
export interface EvidenceShape {
  /** `"ocr-caption"` on the text-only path. `NurtureApiTestResult.evidenceMode`. */
  evidenceMode?: string;
  /** `"grounded-vision"` or `"ocr-caption"`. `NurtureCommentAttempt.source`. */
  source?: string;
  /** Different frames the sheet carried. `undefined` on rows written before it was recorded. */
  distinctFrames?: number;
}

/**
 * This replaces the constant string `"3-frame vision"`, which was false on any still card. A
 * photo post publishes the same picture on every sample — measured 23/08/2026 on
 * ce0717171c2a64d50d, three screencaps 600 ms apart of one photo post differed only inside
 * y 16..49, the animated network icon, and were pixel-identical everywhere below it.
 *
 * Saying "3 khung" taught the operator to read a low `bằng chứng` score as a bad model, when
 * the honest reading is that there was one frame of evidence to ground on. Those are different
 * problems: one is a model to change, the other is a post with nothing more to see.
 */
export function evidenceLabel(result: EvidenceShape): string {
  if (result.evidenceMode === "ocr-caption" || result.source === "ocr-caption") {
    // No picture is sent on this path at all, so any frame count would be noise.
    return "OCR caption + text";
  }
  if (result.distinctFrames === undefined) {
    // Rows written before the count existed. Not "1", not "3" — unknown.
    return "chưa ghi số khung";
  }
  if (result.distinctFrames <= 1) {
    return "1 khung — bài tĩnh, không có chuyển động";
  }
  return `${result.distinctFrames} khung khác nhau`;
}
