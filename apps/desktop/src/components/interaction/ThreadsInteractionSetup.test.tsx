import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import { ThreadsInteractionSetup } from "./ThreadsInteractionSetup";

afterEach(() => { cleanup(); localStorage.clear(); });
it("persists an explicit Threads draft and reports errors without exposing execution", () => {
  const view = render(<ThreadsInteractionSetup devices={[]} labels={new Map()} />);
  fireEvent.click(screen.getByText("Tương tác Threads · bản nháp riêng"));
  fireEvent.click(screen.getByRole("button", { name: "Thêm dòng Threads" }));
  fireEvent.change(screen.getByLabelText("Username Threads"), { target: { value: "@myaccount" } });
  fireEvent.change(screen.getByLabelText("Link bài Threads"), { target: { value: "https://threads.com/@a/post/ABC" } });
  fireEvent.change(screen.getByLabelText("Nội dung trả lời hoặc trích dẫn"), { target: { value: "Nội dung riêng" } });
  fireEvent.click(screen.getByRole("button", { name: "Kiểm tra bản nháp Threads" }));
  expect(screen.getByText(/Dòng 1: chọn máy Android/)).toBeVisible();
  expect(screen.queryByRole("button", { name: /Chạy|Lưu lịch/ })).toBeNull();
  view.unmount();
  render(<ThreadsInteractionSetup devices={[]} labels={new Map()} />);
  fireEvent.click(screen.getByText("Tương tác Threads · bản nháp riêng"));
  expect(screen.getByLabelText("Nội dung trả lời hoặc trích dẫn")).toHaveValue("Nội dung riêng");
  expect(screen.queryByText(/Dòng 1: chọn máy Android/)).toBeNull();
});
