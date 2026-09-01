import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { JobsPanel } from "./JobsPanel";

vi.mock("../api", () => ({
  cancelJob: vi.fn(async () => undefined),
  exampleScript: vi.fn(async () => '{"steps":[]}'),
  runScript: vi.fn(async () => undefined),
}));

describe("JobsPanel page chrome", () => {
  it("leaves the page title to the topbar", () => {
    render(
      <JobsPanel
        jobs={[]}
        devices={[]}
        selectedUdids={[]}
        onSelectUdids={() => undefined}
        onRefresh={() => undefined}
      />,
    );

    expect(screen.queryByRole("heading", { level: 2 })).toBeNull();
  });

  it("shows loading and a retryable list error instead of an empty history", () => {
    const refresh = vi.fn();
    const base = {
      jobs: [],
      devices: [],
      selectedUdids: [],
      onSelectUdids: () => undefined,
      onRefresh: refresh,
    };
    const { rerender } = render(<JobsPanel {...base} loading />);
    expect(screen.getByRole("status")).toHaveTextContent("Đang tải lịch sử tác vụ");
    expect(screen.queryByText("Chưa có tác vụ")).toBeNull();

    rerender(<JobsPanel {...base} loadError="database is locked" />);
    expect(screen.getByRole("alert")).toHaveTextContent("database is locked");
    fireEvent.click(screen.getByRole("button", { name: "Thử lại lịch sử" }));
    expect(refresh).toHaveBeenCalledTimes(1);
  });
});
