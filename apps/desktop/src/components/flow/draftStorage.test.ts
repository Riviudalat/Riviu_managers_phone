import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { newFlowDocument } from "./editorState";
import {
  clearDraft,
  flowDraftKey,
  FlowDraftWriter,
  loadDraft,
  saveDraft,
} from "./draftStorage";

describe("flow draft storage", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("round-trips only the versioned local draft envelope", () => {
    const document = newFlowDocument("Stored");
    document.viewport = { x: 12, y: -4, zoom: 1.25 };
    const stored = saveDraft(document, localStorage, () => new Date("2026-07-31T01:02:03Z"));

    expect(JSON.parse(localStorage.getItem(flowDraftKey(document.id)) ?? "null")).toEqual({
      schemaVersion: 1,
      flowId: document.id,
      baseRevision: 0,
      document,
      savedAt: "2026-07-31T01:02:03.000Z",
    });
    expect(loadDraft(document.id)).toEqual(stored);

    const loaded = loadDraft(document.id);
    if (loaded === null) throw new Error("fixture draft was not loaded");
    loaded.document.name = "Mutated clone";
    expect(loadDraft(document.id)?.document.name).toBe("Stored");
  });

  it("rejects drafts with another schema, key identity, revision, or malformed data", () => {
    const document = newFlowDocument("Fixture");
    const key = flowDraftKey(document.id);
    for (const value of [
      { schemaVersion: 99, flowId: document.id },
      {
        schemaVersion: 1,
        flowId: "another-flow",
        baseRevision: 0,
        document,
        savedAt: "2026-07-31T00:00:00.000Z",
      },
      {
        schemaVersion: 1,
        flowId: document.id,
        baseRevision: 7,
        document,
        savedAt: "2026-07-31T00:00:00.000Z",
      },
    ]) {
      localStorage.setItem(key, JSON.stringify(value));
      expect(loadDraft(document.id)).toBeNull();
    }
    localStorage.setItem(key, "{not-json");
    expect(loadDraft(document.id)).toBeNull();
  });

  it("rejects non-finite document numbers after JSON decoding", () => {
    const document = newFlowDocument("Finite");
    const encoded = JSON.stringify({
      schemaVersion: 1,
      flowId: document.id,
      baseRevision: 0,
      document: { ...document, viewport: { ...document.viewport, zoom: null } },
      savedAt: "2026-07-31T00:00:00.000Z",
    });
    localStorage.setItem(flowDraftKey(document.id), encoded);
    expect(loadDraft(document.id)).toBeNull();
  });

  it("debounces writes for 300 ms and persists only the newest bounded pending draft", () => {
    vi.useFakeTimers();
    const initial = newFlowDocument("First");
    const latest = { ...structuredClone(initial), name: "Latest" };
    const writer = new FlowDraftWriter(localStorage);

    writer.schedule(initial);
    vi.advanceTimersByTime(299);
    expect(loadDraft(initial.id)).toBeNull();
    writer.schedule(latest);
    vi.advanceTimersByTime(299);
    expect(loadDraft(initial.id)).toBeNull();
    vi.advanceTimersByTime(1);
    expect(loadDraft(initial.id)?.document.name).toBe("Latest");
  });

  it("cancels pending writes and clears only the named flow draft", () => {
    vi.useFakeTimers();
    const first = newFlowDocument("First");
    const second = newFlowDocument("Second");
    saveDraft(first);
    saveDraft(second);
    const writer = new FlowDraftWriter(localStorage);
    writer.schedule({ ...structuredClone(first), name: "Pending" });
    writer.cancel();
    vi.advanceTimersByTime(300);
    expect(loadDraft(first.id)?.document.name).toBe("First");

    clearDraft(first.id);
    expect(loadDraft(first.id)).toBeNull();
    expect(loadDraft(second.id)?.document.name).toBe("Second");
  });
});
