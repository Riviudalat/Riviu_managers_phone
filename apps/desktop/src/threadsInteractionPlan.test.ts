import { expect, it } from "vitest";
import { checkThreadsPlan, decodeThreadsDraft, parseThreadsTarget, type ThreadsInteractionRow } from "./threadsInteractionPlan";

const row: ThreadsInteractionRow = { id: "one", udid: "phone", account: "@MyAccount", url: "https://www.threads.net/@author/post/Ab_C-1?x=tracking", action: "reply", text: "Nội dung của tôi" };

it("normalizes direct links without treating author as the acting account", () => {
  const result = checkThreadsPlan({ rows: [row], runAt: "" }, ["phone"]);
  expect(result.issues).toEqual([]);
  expect(result.rows[0]).toMatchObject({ account: "myaccount", targetKey: "Ab_C-1", url: "https://www.threads.com/@author/post/Ab_C-1", text: row.text });
  expect(result.canExecute).toBe(false);
});

it.each([
  "https://threads.com.evil.test/@a/post/abc", "http://threads.com/@a/post/abc",
  "https://name:secret@threads.com/@a/post/abc", "https://threads.com:8443/@a/post/abc",
  "https://threads.com/@a", "https://threads.com/t/abc", "https://threads.com/@a/post/abc/extra",
])("refuses non-direct or unsafe target %s", url => {
  expect(() => parseThreadsTarget(url)).toThrow();
});

it("detects duplicate actions across domains, author aliases and phones for the same account", () => {
  const result = checkThreadsPlan({ runAt: "", rows: [row, { ...row, id: "two", udid: "other", account: "myaccount", url: "https://threads.com/@renamed/post/Ab_C-1" }] }, ["phone", "other"]);
  expect(result.issues).toContain("Dòng 2: trùng tài khoản, bài và hành động với dòng trước.");
});

it("allows distinct accounts and actions on the same target", () => {
  expect(checkThreadsPlan({ runAt: "", rows: [row, { ...row, id: "two", account: "second" }, { ...row, id: "three", action: "like", text: "" }] }, ["phone"]).issues).toEqual([]);
});

it("blocks missing content, unavailable phones, invalid account and elapsed schedule", () => {
  const result = checkThreadsPlan({ runAt: "2020-01-01T10:00", rows: [{ ...row, account: "not a handle", text: " " }] }, [], Date.parse("2026-01-01"));
  expect(result.issues).toHaveLength(4);
});

it("keeps content when changing to like and flags it instead of silently dropping it", () => {
  const result = checkThreadsPlan({ runAt: "", rows: [{ ...row, action: "like" }] }, ["phone"]);
  expect(result.issues[0]).toContain("không gửi nội dung");
  expect(result.rows[0].text).toBe(row.text);
});

it("counts emoji as Unicode characters within the draft limit", () => {
  expect(checkThreadsPlan({ rows: [{ ...row, text: "😀".repeat(500) }], runAt: "" }, ["phone"]).issues).toEqual([]);
  expect(checkThreadsPlan({ rows: [{ ...row, text: "😀".repeat(501) }], runAt: "" }, ["phone"]).issues[0]).toContain("500");
});

it("rejects corrupt stored drafts and never restores approval", () => {
  expect(decodeThreadsDraft({ rows: [null], runAt: "" }).rows).toEqual([]);
  expect(decodeThreadsDraft({ rows: [row, row], runAt: "" }).rows).toEqual([]);
  expect(decodeThreadsDraft({ rows: [{ ...row, action: "toString" }], runAt: "" }).rows).toEqual([]);
  expect(decodeThreadsDraft({ rows: [row], runAt: "", canExecute: true })).toEqual({ rows: [row], runAt: "" });
});
