import { fireEvent, render, screen, within } from "@testing-library/react";
import { expect, it, vi } from "vitest";

import { ALL_DEVICES_TAB } from "../deviceGroups";
import { ControlCenterRail } from "./ControlCenterRail";

vi.mock("./ControlStreamSettings", () => ({ ControlStreamSettings: () => null }));

const machines = [
  { id: "a", number: 1, name: "A", selected: true, connection: "usb" as const },
  { id: "b", number: 2, name: "B", selected: false, connection: "usb" as const },
  { id: "c", number: 3, name: "C", selected: false, connection: "wifi" as const },
  { id: "d", number: 4, name: "D", selected: false, connection: "mock" as const },
];
const groups = [
  { id: ALL_DEVICES_TAB, label: "Tất cả", count: 4 },
  { id: "work", label: "Nhóm làm việc", count: 2, udids: ["a", "c"] },
];

function rail(group: string, onConnection = vi.fn()) {
  return <ControlCenterRail
    tileWidth={180} onTileWidth={vi.fn()} connection="all" onConnection={onConnection}
    groups={groups} group={group} onGroup={vi.fn()} machines={machines}
    onSelect={vi.fn()} onSettings={vi.fn()} pinned onPinnedChange={vi.fn()}
  />;
}

it("shows connection counts for the current group and keeps filtering explicit", () => {
  const onConnection = vi.fn();
  const { rerender } = render(rail(ALL_DEVICES_TAB, onConnection));
  const filters = within(screen.getByRole("group", { name: "Lọc kết nối" }));

  expect(filters.getByRole("button", { name: "Tất cả · 4 máy" })).toBeTruthy();
  expect(filters.getByRole("button", { name: "USB · 2 máy" })).toBeTruthy();
  expect(filters.getByRole("button", { name: "WIFI · 1 máy" })).toBeTruthy();

  rerender(rail("work", onConnection));
  expect(filters.getByRole("button", { name: "Tất cả · 2 máy" })).toBeTruthy();
  expect(filters.getByRole("button", { name: "USB · 1 máy" })).toBeTruthy();
  expect(filters.getByRole("button", { name: "WIFI · 1 máy" })).toBeTruthy();
  fireEvent.click(filters.getByRole("button", { name: "USB · 1 máy" }));
  expect(onConnection).toHaveBeenCalledWith("usb");
});

it("shows zero for an empty group without changing the selected connection", () => {
  render(<ControlCenterRail
    tileWidth={180} onTileWidth={vi.fn()} connection="wifi" onConnection={vi.fn()}
    groups={[...groups, { id: "empty", label: "Trống", count: 0, udids: [] }]}
    group="empty" onGroup={vi.fn()} machines={machines}
    onSelect={vi.fn()} onSettings={vi.fn()} pinned onPinnedChange={vi.fn()}
  />);
  const filters = within(screen.getByRole("group", { name: "Lọc kết nối" }));
  expect(filters.getByRole("button", { name: "Tất cả · 0 máy" })).toBeTruthy();
  expect(filters.getByRole("button", { name: "USB · 0 máy" })).toBeTruthy();
  expect(filters.getByRole("button", { name: "WIFI · 0 máy" }).getAttribute("aria-pressed")).toBe("true");
});
