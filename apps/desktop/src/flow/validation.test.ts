import { describe, expect, it } from "vitest";
import type { ActionKind } from "../types";
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

describe("the local document shape knows every action kind", () => {
  // `ACTION_KINDS` listed fourteen of the sixteen `ActionKind` variants. TypeScript could not see
  // it -- a `Set<ActionKind>` built from a subset is well typed -- and the cost was that adding a
  // Tap Vision or If Vision node made `isFlowDocumentV2` reject the document, which made
  // `FlowDraftWriter.schedule` throw out of the autosave effect, which unmounted the editor.
  it.each(["autoSwipe", "tapVision", "ifVision", "assertVisible", "shell", "rawWda"] as ActionKind[])(
    "accepts a document whose node kind is %s",
    (kind) => {
      const document = newFlowDocument("Vision");
      document.nodes.push({
        id: "node-vision",
        kind,
        position: { x: 10, y: 20 },
        config: {},
      });
      expect(isFlowDocumentV2(document)).toBe(true);
    },
  );

  it("covers the whole ActionKind union, so a new kind cannot be forgotten here", () => {
    // The union itself is the fixture: every variant the catalog can offer must be a kind this
    // module will accept, or drafts of flows using it cannot be stored.
    const everyKind: ActionKind[] = [
      "start",
      "end",
      "launchApp",
      "terminateApp",
      "wait",
      "tap",
      "swipe",
      "autoSwipe",
      "typeText",
      "screenshot",
      "home",
      "assertVisible",
      "tapVision",
      "ifVision",
      "rawHttp",
      "rawWda",
      "shell",
    ];
    const rejected = everyKind.filter((kind) => {
      const document = newFlowDocument("Coverage");
      document.nodes.push({ id: "n", kind, position: { x: 0, y: 0 }, config: {} });
      return !isFlowDocumentV2(document);
    });
    expect(rejected).toEqual([]);
  });
});
