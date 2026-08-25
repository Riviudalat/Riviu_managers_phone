import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";

import { loadQuickPhrases } from "../../quickPhrases";
import { useQuickPhrases } from "./useQuickPhrases";

/**
 * The saved-snippet form, reachable without the focus overlay around it.
 *
 * `quickPhrases.ts` already owns and tests what a valid phrase is. What had no test was the
 * form: whether a refused save keeps what the operator typed, and whether an accepted one is
 * actually written to storage rather than only to React state.
 */
describe("useQuickPhrases", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  it("keeps what was typed when the save is refused", () => {
    // Clearing the fields on a refusal would throw away the text and show an error about
    // text that is no longer on screen.
    const { result } = renderHook(() => useQuickPhrases());
    act(() => result.current.setName("tên"));
    act(() => result.current.setContent("   "));
    act(() => result.current.save());

    expect(result.current.error).toBeTruthy();
    expect(result.current.phrases).toEqual([]);
    expect(result.current.name).toBe("tên");
    expect(result.current.content).toBe("   ");
  });

  it("clears the form only once the phrase is accepted", () => {
    const { result } = renderHook(() => useQuickPhrases());
    act(() => result.current.setName("chào"));
    act(() => result.current.setContent("xin chào"));
    act(() => result.current.save());

    expect(result.current.error).toBeNull();
    expect(result.current.phrases.map((p) => p.content)).toEqual(["xin chào"]);
    expect(result.current.name).toBe("");
    expect(result.current.content).toBe("");
  });

  it("writes through to storage, not just to state", () => {
    // The overlay is unmounted every time the operator closes it, so a phrase that only
    // reached React state is a phrase that vanishes on the next open.
    const { result } = renderHook(() => useQuickPhrases());
    act(() => result.current.setContent("giữ lại"));
    act(() => result.current.save());

    expect(loadQuickPhrases().map((p) => p.content)).toEqual(["giữ lại"]);
  });

  it("removing one persists too", () => {
    const { result } = renderHook(() => useQuickPhrases());
    act(() => result.current.setContent("một"));
    act(() => result.current.save());
    act(() => result.current.setContent("hai"));
    act(() => result.current.save());
    expect(result.current.phrases).toHaveLength(2);

    act(() => result.current.remove(result.current.phrases[0].id));

    expect(result.current.phrases.map((p) => p.content)).toEqual(["hai"]);
    expect(loadQuickPhrases().map((p) => p.content)).toEqual(["hai"]);
  });

  it("starts from what storage already holds", () => {
    const first = renderHook(() => useQuickPhrases());
    act(() => first.result.current.setContent("từ trước"));
    act(() => first.result.current.save());

    const second = renderHook(() => useQuickPhrases());
    expect(second.result.current.phrases.map((p) => p.content)).toEqual(["từ trước"]);
  });
});
