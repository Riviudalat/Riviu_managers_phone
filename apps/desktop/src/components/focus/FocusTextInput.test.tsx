import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { deviceTypeText, groupInput } from "../../api";
import { FocusTextInput } from "./FocusTextInput";

vi.mock("../../api", () => ({ deviceTypeText: vi.fn(), groupInput: vi.fn() }));
const runBusy = vi.fn(async (work: () => Promise<void>) => { await work(); return true; });
const reportGroup = vi.fn();
const props = { udid: "phone-a", targets: ["phone-a"], ready: true, busy: false, runBusy, reportGroup };

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(deviceTypeText).mockResolvedValue(undefined);
  vi.mocked(groupInput).mockResolvedValue({ completedUdids: [], skipped: [] });
});
afterEach(cleanup);

describe("FocusStream text entry", () => {
  it("sends Unicode only on Enter and stops the key reaching overlay shortcuts", async () => {
    render(<FocusTextInput {...props} />);
    const input = screen.getByRole("textbox", { name: "Nhập chữ vào máy" });
    fireEvent.change(input, { target: { value: "Đà Lạt đẹp quá" } });
    expect(deviceTypeText).not.toHaveBeenCalled();
    const overlayKey = vi.fn(); document.body.addEventListener("keydown", overlayKey);
    try {
      fireEvent.keyDown(input, { key: "Enter" });
      await waitFor(() => expect(deviceTypeText).toHaveBeenCalledExactlyOnceWith("phone-a", "Đà Lạt đẹp quá"));
      expect(overlayKey).not.toHaveBeenCalled();
      expect(input).toHaveValue("");
    } finally { document.body.removeEventListener("keydown", overlayKey); }
  });

  it("does not send blank text, unfinished IME composition, or Shift+Enter", () => {
    render(<FocusTextInput {...props} />);
    const input = screen.getByRole("textbox", { name: "Nhập chữ vào máy" });
    fireEvent.change(input, { target: { value: "   " } });
    expect(screen.getByRole("button", { name: "Gửi chữ" })).toBeDisabled();
    fireEvent.keyDown(input, { key: "Enter" });
    fireEvent.change(input, { target: { value: "Việt Nam" } });
    fireEvent.keyDown(input, { key: "Enter", isComposing: true });
    fireEvent.keyDown(input, { key: "Enter", shiftKey: true });
    expect(deviceTypeText).not.toHaveBeenCalled();
  });

  it("does not overlap another operation or use a session before it is ready", async () => {
    let release!: () => void;
    vi.mocked(deviceTypeText).mockReturnValue(new Promise<void>(resolve => { release = resolve; }));
    const view = render(<FocusTextInput {...props} ready={false} />);
    const input = screen.getByRole("textbox", { name: "Nhập chữ vào máy" });
    fireEvent.change(input, { target: { value: "hello" } });
    expect(screen.getByRole("button", { name: "Gửi chữ" })).toBeDisabled();
    view.rerender(<FocusTextInput {...props} busy />);
    expect(screen.getByRole("button", { name: "Gửi chữ" })).toBeDisabled();
    view.rerender(<FocusTextInput {...props} />);
    fireEvent.click(screen.getByRole("button", { name: "Gửi chữ" }));
    fireEvent.keyDown(input, { key: "Enter" });
    expect(deviceTypeText).toHaveBeenCalledTimes(1);
    release();
    await waitFor(() => expect(input).toHaveValue(""));
  });

  it("keeps text edited during an in-flight send instead of clearing the newer draft", async () => {
    let release!: () => void;
    vi.mocked(deviceTypeText).mockReturnValue(new Promise<void>(resolve => { release = resolve; }));
    render(<FocusTextInput {...props} />);
    const input = screen.getByRole("textbox", { name: "Nhập chữ vào máy" });
    fireEvent.change(input, { target: { value: "Tin thứ nhất" } });
    fireEvent.click(screen.getByRole("button", { name: "Gửi chữ" }));
    fireEvent.change(input, { target: { value: "Tin thứ hai" } });
    release();
    await waitFor(() => expect(screen.getByRole("button", { name: "Gửi chữ" })).toBeEnabled());
    expect(input).toHaveValue("Tin thứ hai");
    expect(deviceTypeText).toHaveBeenCalledExactlyOnceWith("phone-a", "Tin thứ nhất");
  });

  it("reports partial group delivery without silently retrying completed devices", async () => {
    vi.mocked(groupInput).mockResolvedValue({ completedUdids: ["phone-a"], skipped: [{ udid: "phone-b", code: "DeviceBusy", currentOwner: "Publish" }] });
    render(<FocusTextInput {...props} targets={["phone-a", "phone-b"]} masterUdid="phone-a" />);
    const input = screen.getByRole("textbox", { name: "Nhập chữ vào máy" });
    fireEvent.change(input, { target: { value: "Xin chào" } });
    fireEvent.click(screen.getByRole("button", { name: "Gửi chữ" }));
    await waitFor(() => expect(groupInput).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({
      udids: ["phone-a", "phone-b"], masterUdid: "phone-a", kind: "type", text: "Xin chào",
    })));
    expect(reportGroup).toHaveBeenCalledOnce();
    expect(await screen.findByRole("alert")).toHaveTextContent("1/2 máy");
    expect(input).toHaveValue("");
    expect(deviceTypeText).not.toHaveBeenCalled();
  });

  it("preserves the text and names a failed group result without retrying", async () => {
    vi.mocked(groupInput).mockResolvedValue({ completedUdids: [], skipped: [
      { udid: "phone-a", code: "DeviceBusy", currentOwner: "Publish" },
      { udid: "phone-b", code: "DeviceBusy", currentOwner: "Publish" },
    ] });
    render(<FocusTextInput {...props} targets={["phone-a", "phone-b"]} />);
    const input = screen.getByRole("textbox", { name: "Nhập chữ vào máy" });
    fireEvent.change(input, { target: { value: "Nội dung cần giữ" } });
    fireEvent.click(screen.getByRole("button", { name: "Gửi chữ" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Không máy nào nhận");
    expect(input).toHaveValue("Nội dung cần giữ");
    expect(groupInput).toHaveBeenCalledTimes(1);
  });
});
