import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { StatusNotice } from "./States";

describe("StatusNotice", () => {
  it.each(["info", "success"] as const)("announces %s non-disruptively", (tone) => {
    render(<StatusNotice tone={tone}>Đã cập nhật</StatusNotice>);
    const notice = screen.getByRole("status");
    expect(notice).toHaveClass(`banner-${tone}`);
    expect(notice).toHaveAttribute("aria-live", "polite");
  });

  it.each(["warning", "error"] as const)("announces %s immediately", (tone) => {
    render(<StatusNotice tone={tone}>Cần xử lý</StatusNotice>);
    const notice = screen.getByRole("alert");
    expect(notice).toHaveClass(`banner-${tone}`);
    expect(notice).toHaveAttribute("aria-live", "assertive");
  });
});
