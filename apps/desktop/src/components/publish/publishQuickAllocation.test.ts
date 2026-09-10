import { describe, expect, it } from "vitest";
import { allocateQuickPosts } from "./publishQuickAllocation";

const base = { sourceIds: ["a", "b", "c"], selectedIds: [], assignments: {}, eligibleIds: ["1", "2", "3", "4"], readyIds: ["1", "2", "3", "4"], picked: [] };
describe("quick publish allocation", () => {
  it("selects source posts and only machines receiving a post", () => {
    expect(allocateQuickPosts(base)).toEqual({ ids: ["a", "b", "c"], assignments: { a: "1", b: "2", c: "3" }, picked: ["1", "2", "3"], missing: [] });
  });
  it("keeps manual pairs and leaves excess posts for another batch", () => {
    expect(allocateQuickPosts({ ...base, assignments: { b: "3" }, picked: ["2"] })).toMatchObject({ ids: ["a", "b"], assignments: { a: "2", b: "3" }, missing: [] });
  });
  it("never borrows a machine when all explicit picks are offline", () => {
    expect(allocateQuickPosts({ ...base, picked: ["offline"] }).ids).toEqual([]);
  });
  it("respects selected posts, scope and source order", () => {
    expect(allocateQuickPosts({ ...base, selectedIds: ["c", "a"], eligibleIds: ["2", "3"] }).assignments).toEqual({ a: "2", c: "3" });
  });
  it("repeating a complete allocation is idempotent", () => {
    const first = allocateQuickPosts(base);
    expect(allocateQuickPosts({ ...base, selectedIds: first.ids, assignments: first.assignments, picked: first.picked })).toEqual(first);
  });
  it("allocates ten posts to ten distinct machines and caps a default batch at 100", () => {
    const sourceIds = Array.from({ length: 110 }, (_, i) => `post${i}`);
    const readyIds = Array.from({ length: 20 }, (_, i) => `device${i}`);
    const ten = allocateQuickPosts({ ...base, sourceIds: sourceIds.slice(0, 10), readyIds, eligibleIds: readyIds });
    expect(Object.keys(ten.assignments)).toHaveLength(10);
    expect(new Set(Object.values(ten.assignments)).size).toBe(10);
    expect(allocateQuickPosts({ ...base, sourceIds }).ids).toHaveLength(4);
    const many = Array.from({ length: 110 }, (_, i) => `device${i}`);
    expect(allocateQuickPosts({ ...base, sourceIds, readyIds: many, eligibleIds: many }).ids).toHaveLength(100);
  });
});
