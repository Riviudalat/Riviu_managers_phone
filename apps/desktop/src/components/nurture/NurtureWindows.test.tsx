import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { NurtureWindows } from "./NurtureWindows";
import type { NurtureSettings, NurtureWindow } from "../../types";

/**
 * The window editor is the only place an operator can see *when* the fleet runs itself, so
 * these tests are about what it says, not only what it stores. Two of them exist because the
 * same mistake was already made once in this feature: a control that means "every phone"
 * while looking like it means "none".
 */

const base = {
  numVideos: 30,
  numRounds: 1,
  likeProb: 20,
  commentProb: 4,
  followProb: 2,
  scheduleEnabled: true,
  scheduleEveryMinutes: 240,
  scheduleDurationMinutes: 150,
  scheduleUdids: [],
} as unknown as NurtureSettings;

// This suite runs without the global auto-cleanup, the same as its neighbours here: each
// test renders the editor fresh, and a leftover tree would make every `getBy` ambiguous.
afterEach(cleanup);

function setup(windows: NurtureWindow[] | undefined, targets: string[] = ["A", "B", "C"]) {
  const patch = vi.fn();
  render(
    <NurtureWindows
      settings={{ ...base, scheduleWindows: windows }}
      patch={patch}
      targets={targets}
    />,
  );
  return patch;
}

const window8to11: NurtureWindow = {
  id: "w-morning",
  startMinute: 8 * 60,
  endMinute: 11 * 60,
  everyMinutes: 60,
  durationMinutes: 20,
  udids: [],
  behaviour: null,
};

describe("NurtureWindows", () => {
  /// With no windows the schedule runs all day, and that is a mode, not an unconfigured one.
  ///
  /// Every settings row written before this editor existed lands here, so the empty state has
  /// to state the consequence — "kể cả ban đêm" — rather than showing two blank numbers.
  it("says what an empty window list actually does", () => {
    setup([]);
    expect(screen.getByLabelText(/^Mỗi \(phút\)/)).toBeVisible();
    expect(screen.getByText(/cả ngày/)).toBeVisible();
    expect(screen.getByText(/ban đêm/)).toBeVisible();
  });

  /// An absent key and an empty list are the same thing, on both sides of the wire.
  it("treats a settings row with no windows key as no windows", () => {
    setup(undefined);
    expect(screen.getByLabelText(/^Mỗi \(phút\)/)).toBeVisible();
  });

  /// **"Tất cả" is written out, because an empty list means every connected phone.**
  ///
  /// This is the one rule in the schedule that has already bitten: the panel's own tooltip
  /// said "chỉ chạy trên những máy đã chọn" while an empty list armed the whole fleet. A
  /// window keeps the rule — twenty phones is often exactly what is wanted — but it may not
  /// keep it silently, so the editor prints the word.
  it("prints an empty phone list as tất cả, never as a blank", () => {
    setup([window8to11]);
    expect(screen.getByText("tất cả")).toBeVisible();
  });

  it("shows the count once a window names phones", () => {
    setup([{ ...window8to11, udids: ["A", "B"] }]);
    expect(screen.getByText("2 máy đã chọn")).toBeVisible();
    expect(screen.queryByText("tất cả")).toBeNull();
  });

  /// Hours go in and out as the clock the operator reads, not as minutes.
  it("edits a window by wall-clock time", () => {
    const patch = setup([window8to11]);
    expect(screen.getByLabelText("Giờ bắt đầu khung 1")).toHaveValue("08:00");

    fireEvent.change(screen.getByLabelText("Giờ bắt đầu khung 1"), {
      target: { value: "09:30" },
    });
    expect(patch).toHaveBeenCalledWith("scheduleWindows", [
      { ...window8to11, startMinute: 9 * 60 + 30 },
    ]);
  });

  /// A window whose end is at or before its start runs through midnight, and says so.
  it("marks a window that wraps past midnight", () => {
    setup([{ ...window8to11, startMinute: 22 * 60, endMinute: 2 * 60 }]);
    expect(screen.getByText("qua đêm")).toBeVisible();
  });

  /// The override is one switch over a complete block, seeded from the panel's own numbers —
  /// so turning it on changes nothing until the operator changes something.
  it("seeds a per-window override from the panel rather than from zero", () => {
    const patch = setup([window8to11]);
    fireEvent.click(screen.getByLabelText(/Cấu hình riêng cho khung này/));
    expect(patch).toHaveBeenCalledWith("scheduleWindows", [
      {
        ...window8to11,
        behaviour: {
          numVideos: 30,
          numRounds: 1,
          likeProb: 20,
          commentProb: 4,
          followProb: 2,
        },
      },
    ]);
  });

  /// A new window starts where the last one ended.
  ///
  /// Two windows on the same hours are legal but the first match wins, so a duplicate pair
  /// would leave the operator watching a second window that never runs.
  it("adds a window after the previous one instead of on top of it", () => {
    const patch = setup([window8to11]);
    fireEvent.click(screen.getByRole("button", { name: /Thêm khung giờ/ }));

    const [, next] = patch.mock.calls[0] as [string, NurtureWindow[]];
    expect(next).toHaveLength(2);
    expect(next[1].startMinute).toBe(window8to11.endMinute);
    expect(next[1].startMinute).toBeGreaterThan(window8to11.startMinute);
  });

  /// The id becomes a settings key holding this window's "next due" mark, and the Rust
  /// validator refuses anything outside this alphabet.
  it("gives a new window an id the backend will accept", () => {
    const patch = setup([]);
    fireEvent.click(screen.getByRole("button", { name: /Thêm khung giờ/ }));

    const [, next] = patch.mock.calls[0] as [string, NurtureWindow[]];
    expect(next[0].id).toMatch(/^[A-Za-z0-9_-]+$/);
  });

  /// Nothing selected on the grid means the button cannot pretend to pick phones.
  it("cannot copy the grid selection when nothing is selected", () => {
    setup([window8to11], []);
    expect(screen.getByRole("button", { name: /Dùng máy đang chọn/ })).toBeDisabled();
  });
});
