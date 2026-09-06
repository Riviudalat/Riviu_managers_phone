import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { interactionImportSheet, interactionReadAccount, interactionReadback } from "../../api";
import { AccountReadControl } from "./AccountReadControl";
import { InteractionReadbackControl } from "./InteractionReadbackControl";
import { InteractionSheetImport } from "./InteractionSheetImport";

vi.mock("../../api", () => ({ interactionImportSheet: vi.fn(), interactionReadAccount: vi.fn(), interactionReadback: vi.fn() }));
beforeEach(() => vi.resetAllMocks());
afterEach(cleanup);

it("shows account mismatch without saving or changing the assigned nick", async () => {
  vi.mocked(interactionReadAccount).mockResolvedValue({ udid: "a", expectedHandle: "expected", observedHandle: "actual", status: "mismatch", checkedAt: "2026-09-06T00:00:00Z", snapshotSha256: "proof" });
  render(<AccountReadControl udid="a" handle="expected" disabled={false} />);
  fireEvent.click(screen.getByRole("button", { name: "Đọc tài khoản từ máy" }));
  await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("Lệch tài khoản · @actual"));
  expect(interactionReadAccount).toHaveBeenCalledWith("a");
});

it("discards account proof when the assigned nick changes during a read", async () => {
  let resolve!: (v: Awaited<ReturnType<typeof interactionReadAccount>>) => void;
  vi.mocked(interactionReadAccount).mockReturnValue(new Promise((r) => { resolve = r; }));
  const { rerender } = render(<AccountReadControl udid="a" handle="old" disabled={false} />);
  fireEvent.click(screen.getByRole("button", { name: "Đọc tài khoản từ máy" }));
  rerender(<AccountReadControl udid="a" handle="new" disabled={false} />);
  await act(async () => resolve({ udid: "a", expectedHandle: "old", observedHandle: "old", status: "matched", checkedAt: "2026-09-06", snapshotSha256: "old" }));
  expect(screen.getByRole("status")).toHaveTextContent("Chưa đối chiếu");
});

it("readback keeps unknown distinct from absent and does not retry actions", async () => {
  vi.mocked(interactionReadback).mockResolvedValue({ assignmentId: "a", targetUrl: "https://www.tiktok.com/@a/video/123", checkedAt: "2026-09-06T00:00:00Z", like: "unknown", save: "saved", snapshotSha256: "proof" });
  render(<InteractionReadbackControl campaignId="c" assignmentId="a" disabled={false} />);
  fireEvent.click(screen.getByRole("button", { name: "Kiểm tra lại kết quả" }));
  expect(await screen.findByRole("status")).toHaveTextContent("Tim chưa rõ; Lưu đang có");
  expect(interactionReadback).toHaveBeenCalledExactlyOnceWith("c", "a");
});

it("Sheet import applies only selected valid unique rows and invalidates when URL changes", async () => {
  const url = "https://www.tiktok.com/@a/photo/123";
  const line = { lineNo: 2, original: url, error: null, target: { originalUrl: url, normalizedUrl: url, targetKey: "content:123", contentId: "123", author: "a", kind: "photo" as const } };
  vi.mocked(interactionImportSheet).mockResolvedValue({ sourceUrl: "https://docs.google.com/spreadsheets/d/abc", column: "D", digest: "proof", rows: [
    { row: 2, line, duplicateOf: null }, { row: 3, line, duplicateOf: 2 },
    { row: 4, line: { lineNo: 4, original: "bad", target: null, error: "invalidUrl" }, duplicateOf: null },
  ] });
  const onApply = vi.fn();
  render(<InteractionSheetImport onApply={onApply} />);
  fireEvent.click(screen.getByText("Nhập từ Google Sheet"));
  fireEvent.change(screen.getByLabelText("Link Sheet"), { target: { value: "https://docs.google.com/spreadsheets/d/abc" } });
  fireEvent.click(screen.getByRole("button", { name: "Đọc Sheet" }));
  await waitFor(() => expect(screen.getByLabelText("Chọn dòng 2")).toBeEnabled());
  expect(screen.getByLabelText("Chọn dòng 3")).toBeDisabled();
  expect(screen.getByLabelText("Chọn dòng 4")).toBeDisabled();
  fireEvent.click(screen.getByLabelText("Chọn dòng 2"));
  fireEvent.click(screen.getByRole("button", { name: "Thêm 1 bài đã chọn" }));
  expect(onApply).toHaveBeenCalledExactlyOnceWith([url]);
  fireEvent.change(screen.getByLabelText("Link Sheet"), { target: { value: "https://docs.google.com/spreadsheets/d/other" } });
  expect(screen.queryByLabelText("Chọn dòng 2")).toBeNull();
});
