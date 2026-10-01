import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import { PublishPreparationProgress } from "./PublishPreparationProgress";
import type { PublishPreflightProgress } from "../../types";

afterEach(cleanup);

it("shows the shared Sheet stage and updates all machines in the same list", () => {
  const udids = Array.from({ length: 12 }, (_, i) => `phone-${i + 1}`);
  const shared: PublishPreflightProgress = {
    requestId: "request", preparationId: "preparation", udid: "", stage: "checkingSheet",
    state: "running", completedChecks: 0, totalChecks: 4, elapsedMs: 1200, revision: 1, error: null,
  };
  const { rerender } = render(<PublishPreparationProgress udids={udids} progress={{ "": shared }}
    startedAt={Date.now()} name={id => id} />);
  expect(screen.getByText("Kiểm tra kết nối Sheet")).toBeVisible();
  expect(screen.getByText("phone-12")).toBeInTheDocument();
  rerender(<PublishPreparationProgress udids={udids}
    progress={{ "": { ...shared, state: "passed", completedChecks: 4, revision: 2 },
      "phone-12": { ...shared, udid: "phone-12", stage: "checkingDevices", state: "passed", completedChecks: 4, revision: 3 } }}
    startedAt={Date.now()} name={id => id} />);
  expect(screen.getByText(/Đã kiểm tra 1\/12 máy/)).toBeVisible();
  expect(screen.getByRole("progressbar", { name: "Tiến độ kiểm tra phone-12" })).toHaveAttribute("aria-valuenow", "100");
});
