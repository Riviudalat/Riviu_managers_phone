import { afterEach, describe, expect, it, vi } from "vitest";
import { newFlowDocument } from "./model";
import { FlowDraftWriter, clearDraft, flowDraftKey, loadDraft, saveDraft } from "./storage";

afterEach(() => {
  localStorage.clear();
  vi.useRealTimers();
});

describe("Flow draft storage", () => {
  it("round-trips the versioned document and preserves canvas layout", () => {
    const document = newFlowDocument("Fixture");
    document.nodes[0].position = { x: 33, y: 44 };
    Object.assign(document.nodes[0], { selected: true, measured: { width: 100, height: 40 } });
    const stored = saveDraft(document, localStorage, () => new Date("2026-07-31T00:00:00Z"));

    expect(stored).toMatchObject({
      schemaVersion: 1,
      flowId: document.id,
      baseRevision: 0,
      savedAt: "2026-07-31T00:00:00.000Z",
    });
    expect(stored.document.nodes[0].position).toEqual({ x: 33, y: 44 });
    expect(loadDraft(document.id)?.document.nodes[0].position).toEqual({ x: 33, y: 44 });
    expect(loadDraft(document.id)?.document.nodes[0]).not.toHaveProperty("selected");
    expect(loadDraft(document.id)?.document.nodes[0]).not.toHaveProperty("measured");
    clearDraft(document.id);
    expect(loadDraft(document.id)).toBeNull();
  });

  it("rejects malformed, cross-flow, and unknown schema envelopes", () => {
    const document = newFlowDocument("Fixture");
    for (const value of [
      "not-json",
      JSON.stringify({ schemaVersion: 99, flowId: document.id }),
      JSON.stringify({
        schemaVersion: 1,
        flowId: "another-flow",
        baseRevision: 0,
        document,
        savedAt: "2026-07-31T00:00:00.000Z",
      }),
    ]) {
      localStorage.setItem(flowDraftKey(document.id), value);
      expect(loadDraft(document.id)).toBeNull();
    }
  });

  it("does not serialize NaN into a nullable JSON value", () => {
    const document = newFlowDocument("Fixture");
    document.nodes[0].config = { durationMs: Number.NaN };
    expect(() => saveDraft(document)).toThrow("finite Flow V2 document");
    expect(localStorage.length).toBe(0);
  });

  it("debounces writes for 300 ms and keeps the latest cloned draft", () => {
    vi.useFakeTimers();
    const first = newFlowDocument("First");
    const writer = new FlowDraftWriter(localStorage);
    writer.schedule(first);
    first.name = "mutated outside writer";
    const latest = { ...first, name: "Latest" };
    writer.schedule(latest);

    vi.advanceTimersByTime(299);
    expect(loadDraft(first.id)).toBeNull();
    vi.advanceTimersByTime(1);
    expect(loadDraft(first.id)?.document.name).toBe("Latest");
  });
});

describe("a draft write that fails on a timer", () => {
  /** A Storage whose `setItem` always throws, the way a full WebView quota does. */
  function refusingStorage(): Storage {
    const inner = new Map<string, string>();
    return {
      get length() {
        return inner.size;
      },
      clear: () => inner.clear(),
      getItem: (key: string) => inner.get(key) ?? null,
      key: (index: number) => [...inner.keys()][index] ?? null,
      removeItem: (key: string) => void inner.delete(key),
      setItem: () => {
        throw new DOMException("QuotaExceededError", "QuotaExceededError");
      },
    } satisfies Storage;
  }

  it("reports the failure instead of losing it inside the timer", () => {
    // `schedule` returns void and the write happens later, so a throw had no caller to reach: the
    // graph stayed dirty on screen, nothing said the recovery draft was never written, and it was
    // gone after a shutdown.
    vi.useFakeTimers();
    const failures: unknown[] = [];
    const writer = new FlowDraftWriter(refusingStorage(), 10, (reason) => failures.push(reason));
    writer.schedule(newFlowDocument("Quota"));
    vi.advanceTimersByTime(20);

    expect(failures).toHaveLength(1);
    expect(String(failures[0])).toContain("QuotaExceededError");
  });

  it("does not report the same failure again on the next flush", () => {
    // Clearing `pending` before the write, not after, is what stops one bad document from being
    // retried by every later flush and reporting forever.
    vi.useFakeTimers();
    const failures: unknown[] = [];
    const writer = new FlowDraftWriter(refusingStorage(), 10, (reason) => failures.push(reason));
    writer.schedule(newFlowDocument("Quota"));
    vi.advanceTimersByTime(20);
    writer.flush();
    writer.flush();
    expect(failures).toHaveLength(1);
  });
});
