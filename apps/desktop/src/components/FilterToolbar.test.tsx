import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { FilterToolbar } from "./FilterToolbar";
import { TILE_ZOOM } from "../zoom";

afterEach(cleanup);

describe("FilterToolbar", () => {
  it("offers a visible tile-size control, because the wheel gesture needs Ctrl and is invisible", () => {
    render(
      <FilterToolbar viewMode="window" onViewMode={vi.fn()} tileWidth={180} onTileWidth={vi.fn()} />,
    );

    const slider = screen.getByLabelText("Cỡ màn hình xem");
    expect(slider).toHaveValue("180");
    // Same bounds as the wheel path, so the two controls cannot disagree about range.
    expect(slider).toHaveAttribute("min", String(TILE_ZOOM.min));
    expect(slider).toHaveAttribute("max", String(TILE_ZOOM.max));
  });

  it("reports a clamped width rather than whatever the input carried", async () => {
    const onTileWidth = vi.fn();
    render(
      <FilterToolbar
        viewMode="window"
        onViewMode={vi.fn()}
        tileWidth={180}
        onTileWidth={onTileWidth}
      />,
    );

    const slider = screen.getByLabelText("Cỡ màn hình xem");
    // fireEvent.change, not userEvent.type: a range input does not take typed keys, and
    // the point here is what the handler forwards, not how a mouse drags a thumb.
    fireEvent.change(slider, { target: { value: "9999" } });
    fireEvent.change(slider, { target: { value: "1" } });

    expect(onTileWidth.mock.calls.map(([width]) => width)).toEqual([
      TILE_ZOOM.max,
      TILE_ZOOM.min,
    ]);
  });

  it("still switches view mode", async () => {
    const onViewMode = vi.fn();
    render(
      <FilterToolbar
        viewMode="window"
        onViewMode={onViewMode}
        tileWidth={180}
        onTileWidth={vi.fn()}
      />,
    );

    await userEvent.click(screen.getByTitle("Danh sách"));

    expect(onViewMode).toHaveBeenCalledWith("list");
  });
});
