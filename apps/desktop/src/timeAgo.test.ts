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

  it("reads the offset, not the digits — which is what the database actually sends", () => {
    // `db/interaction.rs` writes `Utc::now().to_rfc3339()`, so every real timestamp carries
    // `+00:00`, and `getHours()` is then supposed to convert it to the operator's wall clock.
    // Every fixture here used to be an offset-less local string, so the one conversion the
    // function depends on was asserted nowhere: someone "fixing" this to `getUTCHours()` would
    // break it by seven hours in Vietnam and every test would stay green.
    //
    // Two spellings of the same instant have to render the same thing, whatever this machine's
    // timezone is.
    expect(timeAgoVi("2026-08-21T02:05:00+00:00", now)).toBe(
      timeAgoVi("2026-08-21T09:05:00+07:00", now),
    );
  });

  it("does not round the clock forward", () => {
    // Ninety minutes used to read "2 giờ trước". Overstating the age sends an operator looking
    // for a run they remember starting straight past it.
    const ninety = new Date(now.getTime() - 90 * 60 * 1000).toISOString();
    expect(timeAgoVi(ninety, now)).toBe("1 giờ trước");
  });

  it("names the year once it is not this one", () => {
    // Without it, a run from 2025-08-21 and one from 2026-08-21 render identically — the one
    // thing an absolute stamp exists to prevent.
    expect(timeAgoVi("2025-08-21T09:05:00", now)).toBe("09:05 21/08/2025");
    expect(timeAgoVi("2026-08-21T09:05:00", now)).toBe("09:05 21/08");
  });

  it("renders nothing rather than Invalid Date for a missing or broken timestamp", () => {
    expect(timeAgoVi(null, now)).toBe("");
    expect(timeAgoVi(undefined, now)).toBe("");
    expect(timeAgoVi("không phải ngày", now)).toBe("");
  });
});
