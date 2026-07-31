import { describe, expect, it } from "vitest";
import { newFlowDocument } from "./model";
import {
  acceptFiniteValueAsNumber,
  isFlowDocumentV2,
  normalizeFlowIssues,
  parseFiniteNumberInput,
  validateDraftNumbers,
} from "./validation";

describe("Flow input validation", () => {
  it("rejects blank, NaN, infinite, fractional-integer, and out-of-range input", () => {
    expect(parseFiniteNumberInput("")).toBeNull();
    expect(parseFiniteNumberInput("NaN")).toBeNull();
    expect(parseFiniteNumberInput("Infinity")).toBeNull();
    expect(parseFiniteNumberInput("1.5", { integer: true })).toBeNull();
    expect(parseFiniteNumberInput("0", { minimum: 1 })).toBeNull();
    expect(parseFiniteNumberInput("5001", { maximum: 5000 })).toBeNull();
    expect(parseFiniteNumberInput("280", { integer: true, minimum: 1 })).toBe(280);
    expect(acceptFiniteValueAsNumber("bad", Number.NaN)).toBeNull();
    expect(acceptFiniteValueAsNumber("3", 3, { integer: true })).toBe(3);
  });

  it("finds non-finite nested config without accepting it as a document", () => {
    const document = newFlowDocument("Fixture");
    document.nodes[0].config = { nested: { duration: Number.NaN } };

    expect(validateDraftNumbers(document)).toEqual([
      expect.objectContaining({ code: "NonFiniteNumber", field: "config" }),
    ]);
    expect(isFlowDocumentV2(document)).toBe(false);
  });

  it("preserves typed command issues and normalizes unknown failures", () => {
    const issue = {
      code: "NodeConfigInvalid",
      message: "duration is invalid",
      nodeId: "node-a",
      field: "durationMs",
    };
    expect(normalizeFlowIssues([issue])).toEqual([issue]);
    expect(normalizeFlowIssues(new Error("network failed"))).toEqual([
      { code: "ValidationTransportFailed", message: "network failed" },
    ]);
  });
});
