import { describe, expect, it } from "vitest";
import { timeAgoVi } from "./timeAgo";

const now = new Date("2026-08-24T14:30:00");

describe("timeAgoVi", () => {
  it("reads relative while that is the useful answer", () => {
    expect(timeAgoVi("2026-08-24T14:29:50", now)).toBe("vừa xong");
    expect(timeAgoVi("2026-08-24T14:27:00", now)).toBe("3 phút trước");
    expect(timeAgoVi("2026-08-24T09:30:00", now)).toBe("5 giờ trước");
  });

  it("switches to a clock time once relative stops helping find a run", () => {
    expect(timeAgoVi("2026-08-21T09:05:00", now)).toBe("09:05 21/08");
  });

  it("treats a clock skew as now rather than as the future", () => {
    expect(timeAgoVi("2026-08-24T14:30:20", now)).toBe("vừa xong");
  });

  it("renders nothing rather than Invalid Date for a missing or broken timestamp", () => {
    expect(timeAgoVi(null, now)).toBe("");
    expect(timeAgoVi(undefined, now)).toBe("");
    expect(timeAgoVi("không phải ngày", now)).toBe("");
  });
});
