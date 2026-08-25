import { describe, expect, it } from "vitest";

import { COMMENT_MODEL_SUGGESTIONS } from "./commentModels";

/**
 * The list is notes for a human, so what matters is that it stays honest: every entry
 * carries a measurement, and nothing in the app treats it as permission.
 */
describe("the measured comment models", () => {
  it("gives every suggestion a dated measurement, not a claim", () => {
    expect(COMMENT_MODEL_SUGGESTIONS.length).toBeGreaterThan(0);
    for (const s of COMMENT_MODEL_SUGGESTIONS) {
      expect(s.model.trim()).not.toBe("");
      expect(s.baseUrl).toMatch(/^https:\/\//);
      // A date is the cheapest proof that somebody actually ran it.
      expect(s.note, `${s.model} needs a dated measurement`).toMatch(/\d{2}\/\d{2}\/\d{4}/);
    }
  });

  it("does not list the same model twice", () => {
    const models = COMMENT_MODEL_SUGGESTIONS.map((s) => s.model.toLowerCase());
    expect(new Set(models).size).toBe(models.length);
  });

  /**
   * The entry that exists to warn rather than to recommend. Keeping a measured *negative* in
   * the list is the point: `deepseek-v4-flash` is one character away from the vision model
   * and silently falls back to an OCR path that cannot read Vietnamese on Windows.
   */
  it("keeps the model that cannot see, and says so", () => {
    const blind = COMMENT_MODEL_SUGGESTIONS.find((s) => s.model === "deepseek-v4-flash");
    expect(blind).toBeDefined();
    expect(blind?.note).toMatch(/KHÔNG đọc được ảnh/);
  });
});
