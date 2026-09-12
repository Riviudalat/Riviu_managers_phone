import { act, cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import { GroupToolsPopup } from "./GroupToolsPopup";
import type { DeviceInfo } from "../types";
import { clearRecording, recordKey, stopRecording } from "../macroStore";

afterEach(() => { cleanup(); stopRecording(); clearRecording(); });

const devices: DeviceInfo[] = [{ udid: "phone-1", name: "Phone one", model: "Pixel", platform: "android", osVersion: "15", connection: "usb", status: "ready", wdaReady: true }];

it("announces the full-fleet default and moves between tools with the keyboard", async () => {
  const user = userEvent.setup();
  render(<GroupToolsPopup devices={devices} selected={[]} onClose={vi.fn()} />);
  expect(screen.getByRole("dialog", { name: "Công cụ nhóm" })).toHaveAttribute("aria-modal", "true");
  expect(screen.getByText("Tất cả 1 máy")).toBeTruthy();
  const textTab = screen.getByRole("tab", { name: "Phân phối văn bản" });
  textTab.focus();
  await user.keyboard("{ArrowRight}");
  const fileTab = screen.getByRole("tab", { name: "Phân phối tệp" });
  expect(fileTab).toHaveFocus();
  expect(fileTab).toHaveAttribute("aria-selected", "true");
  expect(screen.getByRole("tabpanel", { name: "Phân phối tệp" }).id).toBe(fileTab.getAttribute("aria-controls"));
  await user.keyboard("{Home}");
  expect(textTab).toHaveFocus();
  expect(textTab).toHaveAttribute("aria-selected", "true");
});

it("labels an explicit selection independently from the full fleet", () => {
  render(<GroupToolsPopup devices={[...devices, { ...devices[0], udid: "phone-2" }]} selected={["phone-2"]} onClose={vi.fn()} />);
  expect(screen.getByText("1 máy đã chọn")).toBeTruthy();
  expect(screen.queryByText("Tất cả 2 máy")).toBeNull();
});

it("preserves the recording editor when the modal is hidden and restored", async () => {
  const onBeginMacro = vi.fn();
  const props = { devices, selected: ["phone-1"], onClose: vi.fn(), onBeginMacro };
  const { rerender } = render(<GroupToolsPopup {...props} />);
  const user = userEvent.setup();
  await user.click(screen.getByRole("tab", { name: "Macro" }));
  await user.type(screen.getByRole("textbox", { name: "Tên macro" }), "Morning routine");
  await user.click(screen.getByRole("spinbutton", { name: "Số vòng lặp" }));
  await user.keyboard("{Control>}a{/Control}3");
  await user.click(screen.getByRole("button", { name: "Bắt đầu ghi" }));
  expect(onBeginMacro).toHaveBeenCalledWith(["phone-1"]);
  rerender(<GroupToolsPopup {...props} visible={false} macroTargets={["phone-1"]} />);
  expect(screen.queryByRole("dialog")).toBeNull();
  act(() => { recordKey("home"); stopRecording(); });
  rerender(<GroupToolsPopup {...props} macroTargets={["phone-1"]} />);
  expect(screen.getByRole("textbox", { name: "Tên macro" })).toHaveValue("Morning routine");
  expect(screen.getByRole("textbox", { name: "Tên macro" })).toHaveFocus();
  expect(screen.getByRole("spinbutton", { name: "Số vòng lặp" })).toHaveValue(3);
  expect(screen.getByRole("button", { name: "Lưu macro" })).toBeEnabled();
});

it("retains an empty explicit recording scope and limits cross-page tools to Macro", async () => {
  const onBeginMacro = vi.fn();
  render(<GroupToolsPopup devices={devices} selected={[]} onClose={vi.fn()} macroTargets={[]} macroOnly onBeginMacro={onBeginMacro} />);
  expect(screen.getAllByRole("tab")).toHaveLength(1);
  expect(screen.getByText("0 máy trong phiên ghi")).toBeVisible();
  await userEvent.click(screen.getByRole("button", { name: "Bắt đầu ghi" }));
  expect(onBeginMacro).toHaveBeenCalledWith([]);
});
