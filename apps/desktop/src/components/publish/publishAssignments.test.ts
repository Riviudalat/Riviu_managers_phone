import { describe, it, expect } from "vitest";
import {
  assignDevice,
  fillAssignments,
  reconcileAssignments,
} from "./publishAssignments";
describe("publish mapping identity", () => {
  it("a starting range fills missing posts without dropping mappings before that range", () => {
    expect(fillAssignments(["a", "b"], { a: "1" }, ["1", "2", "3"], 2)).toEqual({ a: "1", b: "3" });
    expect(fillAssignments(["a", "b", "c"], { a: "1" }, ["1", "2", "3"], 2)).toBeNull();
  });
  it("swaps both endpoints while keeping all other pairs", () =>
    expect(assignDevice({ a: "1", b: "2", c: "3" }, "a", "2")).toEqual({
      a: "2",
      b: "1",
      c: "3",
    }));
  it("returns displaced content to unassigned when source had no machine", () =>
    expect(assignDevice({ b: "2", c: "3" }, "a", "2")).toEqual({
      a: "2",
      c: "3",
    }));
  it("removal never shifts sibling devices", () =>
    expect(
      reconcileAssignments(["b", "c"], { a: "1", b: "2", c: "3" }, [
        "1",
        "2",
        "3",
      ]),
    ).toEqual({ b: "2", c: "3" }));
  it("does not partially assign on insufficient capacity", () =>
    expect(fillAssignments(["a", "b"], {}, ["1"])).toBeNull());
  it("autofill preserves manual mapping", () =>
    expect(fillAssignments(["a", "b"], { a: "3" }, ["1", "2", "3"])).toEqual({
      a: "3",
      b: "1",
    }));
  it("disconnect removes only affected machine and rejects duplicates", () =>
    expect(
      reconcileAssignments(["a", "b", "c"], { a: "1", b: "1", c: "3" }, ["1"]),
    ).toEqual({ a: "1" }));
});
