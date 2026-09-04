import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import { describe, expect, it } from "vitest";

import { GroupTabs } from "./GroupTabs";

describe("GroupTabs", () => {
  it("moves focus and selection together with arrow, Home and End", async () => {
    function Fixture() {
      const [active, setActive] = useState("all");
      return (
        <GroupTabs
          active={active}
          onSelect={setActive}
          tabs={[
            { id: "all", label: "Tất cả", count: 20, color: null },
            { id: "north", label: "Kệ Bắc", count: 8, color: "red" },
            { id: "south", label: "Kệ Nam", count: 12, color: null },
          ]}
        />
      );
    }
    render(<Fixture />);

    const all = screen.getByRole("tab", { name: /Tất cả/ });
    const north = screen.getByRole("tab", { name: /Kệ Bắc/ });
    const south = screen.getByRole("tab", { name: /Kệ Nam/ });
    all.focus();
    fireEvent.keyDown(all, { key: "ArrowRight" });
    await waitFor(() => expect(north).toHaveAttribute("aria-selected", "true"));
    expect(north).toHaveFocus();
    fireEvent.keyDown(north, { key: "End" });
    await waitFor(() => expect(south).toHaveFocus());
    fireEvent.keyDown(south, { key: "Home" });
    await waitFor(() => expect(all).toHaveFocus());
  });
});
