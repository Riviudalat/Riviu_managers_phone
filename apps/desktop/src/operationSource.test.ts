import { describe, expect, it } from "vitest";
import { operationSourceFor, operationSourcePage } from "./operationSource";
import type { OperationRunSummary } from "./types";

describe("operation source navigation", () => {
  it.each([
    ["script", "jobs"], ["flow", "scripts"], ["orchestration", "scripts"], ["nurture", "nurture"],
    ["interaction", "interaction"], ["publish", "publish"], ["appInstall", "apps"], ["materialTransfer", "material"],
  ] as const)("keeps %s identity and opens %s", (kind, page) => {
    const source = operationSourceFor({ id: `${kind}:exact`, sourceId: "exact", kind } as OperationRunSummary);
    expect(source).toEqual({ operationId: `${kind}:exact`, sourceId: "exact", kind });
    expect(operationSourcePage(source)).toBe(page);
  });
});
