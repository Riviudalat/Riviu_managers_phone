import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { InfoDot } from "./InfoDot";

describe("InfoDot", () => {
  it("exposes an accessible button and opens the tooltip from keyboard focus", () => {
    render(<InfoDot of="Tần suất" what="Áp dụng cho lần chạy kế tiếp" />);

    const trigger = screen.getByRole("button", { name: "Giải thích Tần suất" });
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();

    fireEvent.focus(trigger);
    const tooltip = screen.getByRole("tooltip");
    expect(tooltip).toHaveTextContent("Áp dụng cho lần chạy kế tiếp");
    expect(trigger).toHaveAttribute("aria-describedby", tooltip.id);

    fireEvent.keyDown(trigger, { key: "Escape" });
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
  });

  it("supports hover and a click-pinned tooltip for pointer and touch input", () => {
    render(<InfoDot of="Chi phí" what="Tổng token đã dùng" />);
    const trigger = screen.getByRole("button", { name: "Giải thích Chi phí" });

    fireEvent.mouseEnter(trigger);
    expect(screen.getByRole("tooltip")).toHaveTextContent("Tổng token đã dùng");
    fireEvent.mouseLeave(trigger);
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();

    fireEvent.click(trigger);
    expect(screen.getByRole("tooltip")).toHaveTextContent("Tổng token đã dùng");
    fireEvent.mouseLeave(trigger);
    expect(screen.getByRole("tooltip")).toBeInTheDocument();
    fireEvent.click(trigger);
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
  });

  it("dismisses a pinned tooltip from the document after focus leaves the trigger", () => {
    render(<InfoDot of="Chi phí" what="Tổng token đã dùng" />);
    const trigger = screen.getByRole("button", { name: "Giải thích Chi phí" });

    fireEvent.click(trigger);
    fireEvent.blur(trigger);
    expect(screen.getByRole("tooltip")).toBeInTheDocument();

    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();

    fireEvent.click(trigger);
    fireEvent.blur(trigger);
    fireEvent.pointerDown(document.body);
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
  });
});
