import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { DeviceGroup, DeviceInfo } from "../types";
import { TargetSelector } from "./TargetSelector";

function device(udid: string, name: string): DeviceInfo {
  return {
    udid,
    name,
    model: "SM-G955F",
    platform: "android",
    osVersion: "9",
    connection: "usb",
    status: "connected",
    wdaReady: false,
  };
}

function group(id: string, name: string, udids: string[]): DeviceGroup {
  return {
    id,
    name,
    color: "#ff6a00",
    udids,
    createdAt: "2026-09-03T00:00:00Z",
  };
}

const devices = [device("serial-a", "ONE-01"), device("serial-b", "ONE-02")];

describe("TargetSelector", () => {
  it("presents all targets without exposing raw device identifiers", () => {
    render(
      <TargetSelector
        devices={devices}
        groups={[]}
        selected={[]}
        onChange={vi.fn()}
        deviceLabel={(entry, index) => `Máy ${index + 1} · ${entry.name}`}
      />,
    );

    expect(screen.getByRole("group", { name: "Phạm vi thiết bị" })).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: "Toàn bộ" })).toBeChecked();
    expect(screen.getByRole("status")).toHaveTextContent("Toàn bộ 2");
    expect(screen.queryByText(/serial-a|serial-b/)).not.toBeInTheDocument();
  });

  it("resolves a group against the current roster when the group is chosen", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(
      <TargetSelector
        devices={devices}
        groups={[
          group("active", "Ca sáng", ["serial-a", "stale-serial"]),
          group("stale", "Máy đã ngắt", ["stale-serial"]),
        ]}
        selected={[]}
        onChange={onChange}
      />,
    );

    await user.click(screen.getByRole("radio", { name: "Nhóm" }));
    const picker = screen.getByRole("combobox", { name: "Chọn nhóm thiết bị" });
    expect(picker).toHaveValue("active");
    expect(onChange).toHaveBeenLastCalledWith(["serial-a"]);
    expect(screen.getByRole("status")).toHaveTextContent("Ca sáng");
    expect(screen.getByRole("option", { name: "Máy đã ngắt (0 máy)" })).toBeEnabled();

    await user.selectOptions(picker, "active");

    expect(onChange).toHaveBeenLastCalledWith(["serial-a"]);
    expect(screen.getByRole("status")).toHaveTextContent("Ca sáng");
    expect(screen.queryByText(/stale-serial|serial-a/)).not.toBeInTheDocument();

    await user.selectOptions(picker, "stale");
    expect(onChange).toHaveBeenLastCalledWith([]);
    expect(screen.getByRole("status")).toHaveTextContent("Máy đã ngắt · 0 máy");
  });

  it("switches mode with the native radio keyboard interaction", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(
      <TargetSelector
        devices={devices}
        groups={[group("active", "Ca sáng", ["serial-a"])]}
        selected={[]}
        onChange={onChange}
      />,
    );

    const modes = screen.getByRole("radiogroup", { name: "Cách chọn thiết bị" });
    const all = screen.getByRole("radio", { name: "Toàn bộ" });
    all.focus();
    await user.keyboard("{ArrowRight}");

    expect(modes).toContainElement(screen.getByRole("radio", { name: "Nhóm" }));
    expect(screen.getByRole("radio", { name: "Nhóm" })).toBeChecked();
    expect(screen.getByRole("combobox", { name: "Chọn nhóm thiết bị" })).toBeInTheDocument();
  });

  it("turns the current fleet into an explicit selection and never emits empty-as-all", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    const { rerender } = render(
      <TargetSelector
        devices={devices}
        groups={[]}
        selected={[]}
        onChange={onChange}
        deviceLabel={(entry, index) => `Máy ${index + 1} · ${entry.name}`}
      />,
    );

    await user.click(screen.getByRole("radio", { name: "Máy cụ thể" }));
    expect(onChange).toHaveBeenLastCalledWith(["serial-a", "serial-b"]);

    rerender(
      <TargetSelector
        devices={devices}
        groups={[]}
        selected={["serial-a", "serial-b"]}
        onChange={onChange}
        deviceLabel={(entry, index) => `Máy ${index + 1} · ${entry.name}`}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent("2 máy cụ thể");
    expect(
      screen.getByRole("group", { name: "Danh sách máy cụ thể" }),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("checkbox", { name: "Máy 1 · ONE-01" }));
    expect(onChange).toHaveBeenLastCalledWith(["serial-b"]);

    rerender(
      <TargetSelector
        devices={devices}
        groups={[]}
        selected={["serial-b"]}
        onChange={onChange}
        deviceLabel={(entry, index) => `Máy ${index + 1} · ${entry.name}`}
      />,
    );
    expect(screen.getByRole("checkbox", { name: "Máy 2 · ONE-02" })).toBeDisabled();
    expect(screen.getByRole("status")).toHaveTextContent("1 máy cụ thể");
  });

  it("renders an explicit empty state when no device is available", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(<TargetSelector devices={[]} groups={[]} selected={[]} onChange={onChange} />);

    expect(screen.getByRole("status")).toHaveTextContent("Toàn bộ 0");
    expect(screen.getByText("Chưa có thiết bị phù hợp.")).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: "Nhóm" })).toBeDisabled();
    expect(screen.getByRole("radio", { name: "Máy cụ thể" })).toBeDisabled();

    await user.click(screen.getByRole("radio", { name: "Toàn bộ" }));
    expect(onChange).not.toHaveBeenCalled();
  });

  it("keeps a controlled empty group distinct from the all-devices scope", () => {
    render(
      <TargetSelector
        devices={devices}
        groups={[group("empty", "Ca trống", ["departed"])]}
        selected={[]}
        targetRef={{ type: "group", groupId: "empty" }}
        onChange={vi.fn()}
        onTargetRefChange={vi.fn()}
      />,
    );

    expect(screen.getByRole("radio", { name: "Nhóm" })).toBeChecked();
    expect(screen.getByRole("status")).toHaveTextContent("Ca trống · 0 máy");
    expect(screen.getByRole("radio", { name: "Toàn bộ" })).not.toBeChecked();
  });

  it("never expands a controlled explicit target when every pinned device leaves the roster", () => {
    render(
      <TargetSelector
        devices={[devices[1]]}
        groups={[]}
        selected={[]}
        targetRef={{ type: "explicit", udids: ["serial-a"] }}
        onChange={vi.fn()}
        onTargetRefChange={vi.fn()}
        deviceLabel={(entry, index) => `Máy ${index + 1} · ${entry.name}`}
      />,
    );

    expect(screen.getByRole("radio", { name: "Máy cụ thể" })).toBeChecked();
    expect(screen.getByRole("status")).toHaveTextContent("0 máy cụ thể");
    expect(screen.getByRole("checkbox", { name: "Máy 1 · ONE-02" })).not.toBeChecked();
  });
});
