import { describe, expect, it } from "vitest";
import {
  addQuickPhrase,
  MAX_QUICK_PHRASES,
  MAX_QUICK_PHRASE_LENGTH,
  parseQuickPhrases,
  removeQuickPhrase,
  type QuickPhrase,
} from "./quickPhrases";

const phrase = (id: string, name = id, content = `content ${id}`): QuickPhrase => ({
  id,
  name,
  content,
});

describe("quick phrases", () => {
  it("names a phrase after its content when no name is given", () => {
    // A nameless entry has to stay findable in the list; an empty row is a button that
    // looks broken.
    const { phrases, error } = addQuickPhrase([], "  ", "xin chào các bạn", "1");
    expect(error).toBeNull();
    expect(phrases[0]).toEqual({ id: "1", name: "xin chào các bạn", content: "xin chào các bạn" });
  });

  it("refuses an empty phrase with a reason rather than saving a blank row", () => {
    const { phrases, error } = addQuickPhrase([], "tên", "   ", "1");
    expect(phrases).toEqual([]);
    expect(error).toBe("Chưa có nội dung để lưu.");
  });

  it("bounds one phrase and the whole book", () => {
    const long = addQuickPhrase([], "", "x".repeat(MAX_QUICK_PHRASE_LENGTH + 1), "1");
    expect(long.error).toContain(String(MAX_QUICK_PHRASE_LENGTH));

    const full = Array.from({ length: MAX_QUICK_PHRASES }, (_, index) =>
      phrase(String(index)),
    );
    const overflow = addQuickPhrase(full, "", "one more", "new");
    expect(overflow.phrases).toHaveLength(MAX_QUICK_PHRASES);
    expect(overflow.error).toContain(String(MAX_QUICK_PHRASES));
  });

  it("trims the content it stores, so a stray newline is not typed onto the phone", () => {
    const { phrases } = addQuickPhrase([], "tên", "  xin chào\n", "1");
    expect(phrases[0].content).toBe("xin chào");
  });

  it("removes by id and leaves the rest in order", () => {
    const book = [phrase("a"), phrase("b"), phrase("c")];
    expect(removeQuickPhrase(book, "b").map((entry) => entry.id)).toEqual(["a", "c"]);
  });

  it("survives anything already in storage rather than throwing", () => {
    // A corrupt preference must not stop the overlay from opening, so every one of these
    // reads as "no phrases yet".
    expect(parseQuickPhrases(null)).toEqual([]);
    expect(parseQuickPhrases("not json")).toEqual([]);
    expect(parseQuickPhrases('{"not":"an array"}')).toEqual([]);
    expect(parseQuickPhrases('[{"id":1}]')).toEqual([]);
    expect(parseQuickPhrases('[{"id":"a","name":"n","content":"c"},null,7]')).toEqual([
      { id: "a", name: "n", content: "c" },
    ]);
  });

  it("caps what it reads back, not only what it writes", () => {
    const stored = JSON.stringify(
      Array.from({ length: MAX_QUICK_PHRASES + 10 }, (_, index) => phrase(String(index))),
    );
    expect(parseQuickPhrases(stored)).toHaveLength(MAX_QUICK_PHRASES);
  });
});
