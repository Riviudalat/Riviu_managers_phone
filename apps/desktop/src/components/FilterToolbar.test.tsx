import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { FilterToolbar } from "./FilterToolbar";

afterEach(cleanup);

describe("FilterToolbar", () => {
  it("switches view mode", async () => {
    const onViewMode = vi.fn();
    render(<FilterToolbar viewMode="window" onViewMode={onViewMode} />);

    await userEvent.click(screen.getByTitle("Danh sách"));

    expect(onViewMode).toHaveBeenCalledWith("list");
  });

  /**
   * The slider is gone at the operator's request: Ctrl + wheel over the grid is the control
   * they want, and the toolbar row was carrying a duplicate of it. Asserted so nobody puts it
   * back by reflex — the zoom range and clamp are tested in `zoom.test.ts`, and the gesture's
   * wiring lives on the grid itself.
   */
  it("carries no tile-size slider", () => {
    render(<FilterToolbar viewMode="window" onViewMode={vi.fn()} />);

    expect(screen.queryByLabelText("Cỡ màn hình xem")).toBeNull();
    expect(document.querySelector("input[type='range']")).toBeNull();
  });
});
