import { describe, expect, it } from "vitest";
import {
  addQuickPhrase,
  DEFAULT_QUICK_GROUP,
  exportQuickPhrases,
  groupsOf,
  importQuickPhrases,
  MAX_QUICK_PHRASES,
  MAX_QUICK_PHRASE_LENGTH,
  parseQuickPhrases,
  phrasesInGroup,
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
    expect(phrases[0]).toEqual({
      id: "1",
      name: "xin chào các bạn",
      content: "xin chào các bạn",
      group: DEFAULT_QUICK_GROUP,
    });
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
      { id: "a", name: "n", content: "c", group: DEFAULT_QUICK_GROUP },
    ]);
  });

  it("caps what it reads back, not only what it writes", () => {
    const stored = JSON.stringify(
      Array.from({ length: MAX_QUICK_PHRASES + 10 }, (_, index) => phrase(String(index))),
    );
    expect(parseQuickPhrases(stored)).toHaveLength(MAX_QUICK_PHRASES);
  });
});

describe("quick phrase groups & import/export", () => {
  const withGroup = (id: string, group: string): QuickPhrase => ({
    id,
    name: id,
    content: `content ${id}`,
    group,
  });

  it("adds a phrase into a normalized group; blank group folds to default", () => {
    const a = addQuickPhrase([], "n", "c", "1", "  Marketing ");
    expect(a.phrases[0].group).toBe("Marketing");
    const b = addQuickPhrase([], "n", "c", "2", "   ");
    expect(b.phrases[0].group).toBe(DEFAULT_QUICK_GROUP);
  });

  it("lists groups with the default first, then first-seen order", () => {
    const book = [withGroup("a", "Sale"), withGroup("b", DEFAULT_QUICK_GROUP), withGroup("c", "Sale")];
    expect(groupsOf(book)).toEqual([DEFAULT_QUICK_GROUP, "Sale"]);
  });

  it("filters phrases by group", () => {
    const book = [withGroup("a", "Sale"), withGroup("b", "Care")];
    expect(phrasesInGroup(book, "Sale").map((p) => p.id)).toEqual(["a"]);
  });

  it("round-trips through export/import, preserving group/name/content and newlines", () => {
    const src = addQuickPhrase([], "Chào", "Xin chào\nBạn khỏe không", "1", "Sale").phrases;
    const text = exportQuickPhrases(src);
    let counter = 0;
    const back = importQuickPhrases([], text, () => `imp-${(counter += 1)}`);
    expect(back.added).toBe(1);
    expect(back.skipped).toBe(0);
    expect(back.phrases[0]).toMatchObject({
      name: "Chào",
      content: "Xin chào\nBạn khỏe không",
      group: "Sale",
    });
  });

  it("imports tolerant line shapes (3 / 2 / 1 columns)", () => {
    const raw = ["Sale\tTên\tNội dung", "Care\tChỉ nội dung", "Không nhóm"].join("\n");
    let n = 0;
    const out = importQuickPhrases([], raw, () => `x${(n += 1)}`);
    expect(out.added).toBe(3);
    expect(out.phrases[0]).toMatchObject({ group: "Sale", name: "Tên", content: "Nội dung" });
    expect(out.phrases[1]).toMatchObject({ group: "Care", content: "Chỉ nội dung" });
    expect(out.phrases[2]).toMatchObject({ group: DEFAULT_QUICK_GROUP, content: "Không nhóm" });
  });

  it("skips blank lines and merges onto the existing book", () => {
    const existing = addQuickPhrase([], "n", "cũ", "0", "Care").phrases;
    let n = 0;
    const out = importQuickPhrases(existing, "\n\nCare\tmới\n\n", () => `y${(n += 1)}`);
    expect(out.added).toBe(1);
    expect(out.phrases).toHaveLength(2);
  });
});
