import { describe, expect, it } from "vitest";
import { allocateQuickPosts } from "./publishQuickAllocation";

const base = { sourceIds: ["a", "b", "c"], selectedIds: [], assignments: {}, eligibleIds: ["1", "2", "3", "4"], readyIds: ["1", "2", "3", "4"], picked: [] };
describe("quick publish allocation", () => {
  it("selects source posts and only machines receiving a post", () => {
    expect(allocateQuickPosts(base)).toEqual({ ids: ["a", "b", "c"], assignments: { a: "1", b: "2", c: "3" }, picked: ["1", "2", "3"], missing: [] });
  });
  it("keeps manual pairs and fills remaining ready machines from the source", () => {
    expect(allocateQuickPosts({ ...base, assignments: { b: "3" }, picked: ["2"] })).toMatchObject({
      ids: ["a", "b", "c"], assignments: { a: "1", b: "3", c: "2" }, missing: [],
    });
  });
  it("ignores stale picked machines when filling capacity", () => {
    expect(allocateQuickPosts({ ...base, picked: ["offline"] }).assignments).toEqual({ a: "1", b: "2", c: "3" });
  });
  it("respects an incomplete explicit post selection", () => {
    expect(allocateQuickPosts({ ...base, selectedIds: ["c", "a"], eligibleIds: ["2", "3"] }).assignments).toEqual({ a: "2", c: "3" });
  });
  it("expands past a complete prior selection when more ready machines appear", () => {
    const sourceIds = Array.from({ length: 20 }, (_, i) => `post${i}`);
    const firstReady = Array.from({ length: 10 }, (_, i) => `device${i}`);
    const first = allocateQuickPosts({ ...base, sourceIds, readyIds: firstReady, eligibleIds: firstReady });
    expect(first.ids).toHaveLength(10);
    const moreReady = Array.from({ length: 14 }, (_, i) => `device${i}`);
    const second = allocateQuickPosts({
      ...base, sourceIds, selectedIds: first.ids, assignments: first.assignments,
      readyIds: moreReady, eligibleIds: moreReady, picked: first.picked,
    });
    expect(second.ids).toHaveLength(14);
    expect(new Set(Object.values(second.assignments)).size).toBe(14);
    for (const id of first.ids) expect(second.assignments[id]).toBe(first.assignments[id]);
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
