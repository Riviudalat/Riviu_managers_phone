import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";

import { NurtureDeviceLog } from "./NurtureDeviceLog";

const sessionLog = vi.hoisted(() => vi.fn());

vi.mock("../../api", () => ({
  nurtureClearSessionLog: vi.fn(async () => undefined),
  nurtureSessionLog: sessionLog,
}));

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

it("shows the shared Vietnamese status while retaining raw engine text as detail", async () => {
  sessionLog.mockResolvedValue([
    {
      at: "2026-09-04T00:00:00Z",
      lastAt: "2026-09-04T00:00:00Z",
      text: "save skip: state unreadable",
      repeats: 1,
    },
  ]);

  render(
    <NurtureDeviceLog
      udid="device-1"
      running={false}
      presentStatus={() => "Bỏ lưu: không đọc được trạng thái"}
    />,
  );

  const status = await screen.findByText("Bỏ lưu: không đọc được trạng thái");
  expect(status).toHaveAttribute("title", "save skip: state unreadable");
  expect(screen.queryByText("save skip: state unreadable")).toBeNull();
});
