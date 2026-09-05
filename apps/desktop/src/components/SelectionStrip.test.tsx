import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { SelectionStrip } from "./SelectionStrip";
import { listGroups } from "../api";
import type { DeviceInfo } from "../types";

vi.mock("../api", () => ({ listGroups: vi.fn() }));

const devices: DeviceInfo[] = [{
  udid: "phone-a",
  name: "Máy quay",
  model: "SM-G950F",
  platform: "android",
  osVersion: "9",
  connection: "usb",
  status: "ready",
  wdaReady: false,
}];

beforeEach(() => {
  vi.mocked(listGroups).mockReset();
});

afterEach(cleanup);

describe("SelectionStrip group state", () => {
  it("never turns an empty group into the legacy all-device scope", async () => {
    vi.mocked(listGroups).mockResolvedValue([{
      id: "empty", name: "Nhóm rỗng", color: "#f97316", udids: [], createdAt: "2026-09-06",
    }]);
    const select = vi.fn();
    render(<SelectionStrip devices={devices} selected={["phone-a"]}
      onSelectAll={vi.fn()} onClear={vi.fn()} onSelectUdids={select} />);
    const option = await screen.findByRole("option", { name: /Nhóm rỗng/ });
    expect(option).toBeDisabled();
    await userEvent.selectOptions(screen.getByRole("combobox", { name: "Chọn theo nhóm" }), "empty");
    expect(select).not.toHaveBeenCalled();
  });

  it("shows a scoped group error and recovers through retry", async () => {
    const selectUdids = vi.fn();
    vi.mocked(listGroups)
      .mockRejectedValueOnce(new Error("group database unavailable"))
      .mockResolvedValueOnce([{
        id: "group-a",
        name: "Máy đăng bài",
        color: "#f97316",
        udids: ["phone-a"],
        createdAt: "2026-09-05T00:00:00Z",
      }]);

    render(
      <SelectionStrip
        devices={devices}
        selected={[]}
        onSelectAll={() => undefined}
        onClear={() => undefined}
        onSelectUdids={selectUdids}
      />,
    );

    expect(await screen.findByRole("alert")).toHaveTextContent("group database unavailable");
    expect(screen.queryByRole("combobox", { name: "Chọn theo nhóm" })).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Thử lại nhóm" }));

    const picker = await screen.findByRole("combobox", { name: "Chọn theo nhóm" });
    await userEvent.selectOptions(picker, "group-a");
    expect(selectUdids).toHaveBeenCalledWith(["phone-a"]);
    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
  });
});
