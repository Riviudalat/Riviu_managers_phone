import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useState } from "react";
import { PublishDialog } from "./PublishDialog";
import { ConfirmHost } from "../ConfirmHost";
import { requestConfirm, resetConfirms } from "../../confirmStore";

beforeEach(() => {
  if (!HTMLDialogElement.prototype.showModal) Object.defineProperty(HTMLDialogElement.prototype, "showModal", { configurable: true, writable: true, value() {} });
  if (!HTMLDialogElement.prototype.close) Object.defineProperty(HTMLDialogElement.prototype, "close", { configurable: true, writable: true, value() {} });
  vi.spyOn(HTMLDialogElement.prototype, "showModal").mockImplementation(function (this: HTMLDialogElement) { this.setAttribute("open", ""); });
  vi.spyOn(HTMLDialogElement.prototype, "close").mockImplementation(function (this: HTMLDialogElement) { this.removeAttribute("open"); });
});
afterEach(() => { cleanup(); resetConfirms(); vi.restoreAllMocks(); });

it("nhường top-layer cho confirm chung rồi trả đúng trigger khi dialog đóng", async () => {
  function Harness() {
    const [open, setOpen] = useState(false);
    return <><button onClick={() => setOpen(true)}>Mở caption</button>{open && <PublishDialog title="Caption" onClose={() => setOpen(false)}>
      <button onClick={() => void requestConfirm({ title: "Xác nhận tác vụ", confirmLabel: "Xác nhận" })}>Yêu cầu xác nhận</button>
    </PublishDialog>}<ConfirmHost/></>;
  }
  render(<Harness/>);
  const origin = screen.getByRole("button", { name: "Mở caption" });
  origin.focus(); fireEvent.click(origin);
  const dialog = screen.getByRole("dialog", { name: "Caption" });
  const inner = screen.getByRole("button", { name: "Yêu cầu xác nhận" }); inner.focus(); fireEvent.click(inner);
  expect(dialog).not.toHaveAttribute("open");
  expect(screen.getByRole("button", { name: "Xác nhận" })).toHaveFocus();
  await act(async () => { fireEvent.click(screen.getByRole("button", { name: "Hủy" })); });
  expect(dialog).toHaveAttribute("open");
  fireEvent.click(screen.getByRole("button", { name: "Đóng" }));
  await waitFor(() => expect(origin).toHaveFocus());
});

it("trigger ẩn dùng fallback ổn định, không lấy focus khỏi confirm đang chờ", async () => {
  const origin = document.createElement("button"), fallback = document.createElement("button");
  document.body.append(origin, fallback); origin.focus();
  const view = render(<><PublishDialog title="Caption" onClose={() => {}} returnFocus={() => origin} fallbackFocus={() => fallback}>Nội dung</PublishDialog><ConfirmHost/></>);
  origin.hidden = true;
  view.rerender(<ConfirmHost/>);
  await waitFor(() => expect(fallback).toHaveFocus());
  view.rerender(<><PublishDialog title="Caption" onClose={() => {}} fallbackFocus={() => fallback}>Nội dung</PublishDialog><ConfirmHost/></>);
  act(() => { void requestConfirm({ title: "Chờ quyết định", confirmLabel: "Quyết định" }); });
  view.rerender(<ConfirmHost/>);
  await act(async () => { await Promise.resolve(); });
  expect(screen.getByRole("button", { name: "Quyết định" })).toHaveFocus();
  origin.remove(); fallback.remove();
});
