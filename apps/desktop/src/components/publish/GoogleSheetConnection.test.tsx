import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { googleSheetsCancel, googleSheetsConnect, googleSheetsLogin, googleSheetsPickFile, googleSheetsStatus, publishSheetCheck, publishSheetGetConfig } from "../../api";
import type { GoogleSheetsStatus, PublishSheetCheckResult } from "../../types";
import { GoogleSheetConnection } from "./GoogleSheetConnection";
import { parseGoogleSheetUrl } from "./googleSheetUrl";
vi.mock("../../api", () => ({ googleSheetsCancel: vi.fn(), googleSheetsConnect: vi.fn(), googleSheetsLogin: vi.fn(), googleSheetsPickFile: vi.fn(), googleSheetsStatus: vi.fn(), publishSheetCheck: vi.fn(), publishSheetGetConfig: vi.fn() }));
const url = "https://docs.google.com/spreadsheets/d/file-a/edit#gid=7";
const status = (patch: Partial<GoogleSheetsStatus> = {}): GoogleSheetsStatus => ({ configured: true, connected: true, active: false, accountId: "account-a", email: "operator@example.test", clientId: "id", pickerConfigured: true, phase: "idle", ...patch });
const active = (patch: Partial<GoogleSheetsStatus> = {}) => status({ active: true, selectedFileId: "file-a", writerId: "writer-a", sheetUrl: url, ...patch });
const checked = (patch: Partial<PublishSheetCheckResult> = {}): PublishSheetCheckResult => ({ sheetUrl: url, spreadsheetId: "file-a", sheetGid: 7, readable: true, connectionVerified: true, reportingReady: true, layout: "internal", columns: [], message: "Đã xác minh", ...patch });
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>(yes => { resolve = yes; }); return { promise, resolve }; }
beforeEach(() => { vi.resetAllMocks(); vi.mocked(googleSheetsStatus).mockResolvedValue(status()); vi.mocked(publishSheetGetConfig).mockResolvedValue({ webhookUrl: "", hasToken: false }); vi.mocked(publishSheetCheck).mockResolvedValue(checked()); vi.mocked(googleSheetsConnect).mockResolvedValue(checked()); vi.mocked(googleSheetsLogin).mockResolvedValue(status()); vi.mocked(googleSheetsCancel).mockResolvedValue(status()); vi.mocked(googleSheetsPickFile).mockResolvedValue(status({ selectedFileId: "file-a" })); });
afterEach(() => { cleanup(); vi.useRealTimers(); });
async function editUrl(value = url) { await waitFor(() => expect(screen.getByRole("button", { name: "Đăng nhập Google" })).toBeEnabled()); fireEvent.change(screen.getByRole("textbox", { name: "Link Google Sheet" }), { target: { value } }); }

describe("compact Google Sheet connection", () => {
  it("shows exactly one URL input and two buttons without old configuration", async () => {
    render(<GoogleSheetConnection />); await editUrl();
    expect(screen.getAllByRole("textbox")).toHaveLength(1); expect(screen.getAllByRole("button")).toHaveLength(2);
    expect(screen.getByRole("button", { name: "Kiểm tra kết nối" })).toBeVisible();
    expect(screen.queryByText(/Apps Script|Client secret|Project number|Chọn bảng từ Google Drive/)).toBeNull();
  });
  it("parses exact Google file and gid with zero only when omitted", () => {
    expect(parseGoogleSheetUrl(url)).toEqual({ spreadsheetId: "file-a", sheetId: 7, url });
    expect(parseGoogleSheetUrl("https://docs.google.com/spreadsheets/d/file-a/edit")).toMatchObject({ sheetId: 0 });
    expect(parseGoogleSheetUrl("https://docs.google.com/spreadsheets/d/file-a/edit?gid=12")).toMatchObject({ sheetId: 12 });
    for (const value of ["https://docs.google.com.evil.test/spreadsheets/d/a/edit", "http://docs.google.com/spreadsheets/d/a/edit", url + "&gid=no", "https://docs.google.com/spreadsheets/d/a/edit?gid=1#gid=2", "https://docs.google.com/spreadsheets/d/a/edit#gid=-1", "https://docs.google.com/spreadsheets/d/a/edit?gid=1&gid=2", "https://docs.google.com/spreadsheets/d/a/edit#gid=2147483648", `https://docs.google.com/spreadsheets/d/${"a".repeat(129)}/edit`])
      expect(parseGoogleSheetUrl(value)).toBeNull();
  });
  it("reads an active exact target without picker or reconnect", async () => {
    vi.mocked(googleSheetsStatus).mockResolvedValue(active({ selectedFileId: "other-picked-file" }));
    const ready = vi.fn(); render(<GoogleSheetConnection onReadyChange={ready} />);
    await waitFor(() => expect(ready).toHaveBeenLastCalledWith(true));
    expect(publishSheetCheck).toHaveBeenCalledExactlyOnceWith(url);
    expect(googleSheetsPickFile).not.toHaveBeenCalled(); expect(googleSheetsConnect).not.toHaveBeenCalled();
  });
  it("connects the selected file to the gid in the typed URL and refreshes status", async () => {
    vi.mocked(googleSheetsStatus).mockResolvedValue(status({ selectedFileId: "file-a" }));
    const ready = vi.fn(); render(<GoogleSheetConnection onReadyChange={ready} />); await editUrl();
    vi.mocked(googleSheetsStatus).mockResolvedValueOnce(status({ selectedFileId: "file-a" })).mockResolvedValueOnce(active());
    fireEvent.click(screen.getByRole("button", { name: "Kiểm tra kết nối" }));
    await waitFor(() => expect(ready).toHaveBeenLastCalledWith(true));
    expect(googleSheetsConnect).toHaveBeenCalledExactlyOnceWith("file-a", 7, true); expect(googleSheetsPickFile).not.toHaveBeenCalled();
  });
  it("opens Picker only for missing file permission and rejects a different chosen file", async () => {
    vi.mocked(googleSheetsPickFile).mockResolvedValue(status({ selectedFileId: "wrong-file" }));
    const ready = vi.fn(); render(<GoogleSheetConnection onReadyChange={ready} />); await editUrl();
    fireEvent.click(screen.getByRole("button", { name: "Kiểm tra kết nối" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("không khớp link");
    expect(screen.getByRole("textbox", { name: "Link Google Sheet" })).toHaveValue(url); expect(googleSheetsConnect).not.toHaveBeenCalled(); expect(ready).toHaveBeenLastCalledWith(false);
  });
  it("waits for matching Picker choice then connects without another button", async () => {
    vi.useFakeTimers(); vi.mocked(googleSheetsPickFile).mockResolvedValue(status({ phase: "picking" }));
    const ready = vi.fn(); render(<GoogleSheetConnection onReadyChange={ready} />); await act(async () => {});
    fireEvent.change(screen.getByRole("textbox", { name: "Link Google Sheet" }), { target: { value: url } });
    vi.mocked(googleSheetsStatus).mockResolvedValueOnce(status()).mockResolvedValueOnce(status({ selectedFileId: "file-a" })).mockResolvedValueOnce(active());
    fireEvent.click(screen.getByRole("button", { name: "Kiểm tra kết nối" })); await act(async () => {});
    expect(screen.getByRole("status")).toHaveTextContent("Chọn đúng bảng trong cửa sổ Google");
    await act(async () => vi.advanceTimersByTimeAsync(1500));
    expect(googleSheetsConnect).toHaveBeenCalledExactlyOnceWith("file-a", 7, true); expect(ready).toHaveBeenLastCalledWith(true);
  });
  it("invalidates a late read after editing, even when the URL changes back", async () => {
    vi.mocked(googleSheetsStatus).mockResolvedValue(active()); const read = deferred<PublishSheetCheckResult>(); vi.mocked(publishSheetCheck).mockReturnValue(read.promise);
    const ready = vi.fn(); render(<GoogleSheetConnection onReadyChange={ready} />); await waitFor(() => expect(publishSheetCheck).toHaveBeenCalled());
    const input = screen.getByRole("textbox", { name: "Link Google Sheet" }); fireEvent.change(input, { target: { value: url + "x" } }); fireEvent.change(input, { target: { value: url } });
    await act(async () => read.resolve(checked())); expect(ready).toHaveBeenLastCalledWith(false); expect(screen.getByRole("button", { name: "Kiểm tra kết nối" })).toBeEnabled();
  });
  it("ignores late login after cancel and suppresses repeated clicks", async () => {
    const login = deferred<GoogleSheetsStatus>(); vi.mocked(googleSheetsLogin).mockReturnValue(login.promise);
    render(<GoogleSheetConnection />); await editUrl(); const button = screen.getByRole("button", { name: "Đăng nhập Google" }); fireEvent.click(button); fireEvent.click(button);
    await waitFor(() => expect(googleSheetsLogin).toHaveBeenCalledOnce());
    fireEvent.click(screen.getByRole("button", { name: "Hủy đăng nhập" })); await waitFor(() => expect(googleSheetsCancel).toHaveBeenCalledOnce());
    await act(async () => login.resolve(active())); expect(publishSheetCheck).not.toHaveBeenCalled(); expect(googleSheetsConnect).not.toHaveBeenCalled();
  });
  it("login does not claim Sheet readiness and missing config stays inline", async () => {
    vi.mocked(googleSheetsStatus).mockResolvedValue(status({ configured: false, connected: false }));
    const ready = vi.fn(); render(<GoogleSheetConnection onReadyChange={ready} />); await editUrl(); fireEvent.click(screen.getByRole("button", { name: "Đăng nhập Google" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Chưa có cấu hình Google trên máy này"); expect(googleSheetsLogin).not.toHaveBeenCalled(); expect(ready).toHaveBeenLastCalledWith(false);
  });
  it("clears readiness when focus refresh observes expired credentials", async () => {
    vi.mocked(googleSheetsStatus).mockResolvedValue(active()); const ready = vi.fn(); render(<GoogleSheetConnection onReadyChange={ready} />);
    await waitFor(() => expect(ready).toHaveBeenLastCalledWith(true));
    vi.mocked(googleSheetsStatus).mockResolvedValue(active({ connected: false, error: "Phiên Google hết hạn" })); fireEvent(window, new Event("focus"));
    await waitFor(() => expect(ready).toHaveBeenLastCalledWith(false)); expect(screen.getByRole("alert")).toHaveTextContent("hết hạn"); expect(screen.getByRole("button", { name: "Đăng nhập Google" })).toBeEnabled();
  });
  it("clears readiness if the active destination changes outside the current form", async () => {
    vi.mocked(googleSheetsStatus).mockResolvedValue(active()); const ready = vi.fn(); render(<GoogleSheetConnection onReadyChange={ready} />);
    await waitFor(() => expect(ready).toHaveBeenLastCalledWith(true));
    vi.mocked(googleSheetsStatus).mockResolvedValue(active({ sheetUrl: "https://docs.google.com/spreadsheets/d/file-a/edit#gid=9" }));
    fireEvent(window, new Event("focus"));
    await waitFor(() => expect(ready).toHaveBeenLastCalledWith(false));
    expect(screen.getByRole("textbox", { name: "Link Google Sheet" })).toHaveValue(url);
  });
  it("polls only the browser flow and stops after the five-minute deadline", async () => {
    vi.useFakeTimers(); vi.mocked(googleSheetsStatus).mockResolvedValue(status({ phase: "authorizing" }));
    render(<GoogleSheetConnection />); await act(async () => {}); fireEvent.click(screen.getByRole("button", { name: "Đăng nhập Google" })); await act(async () => {});
    await act(async () => vi.advanceTimersByTimeAsync(301_000));
    expect(googleSheetsCancel).toHaveBeenCalledOnce(); expect(screen.getByRole("alert")).toHaveTextContent("hết thời gian");
    const calls = vi.mocked(googleSheetsStatus).mock.calls.length; await act(async () => vi.advanceTimersByTimeAsync(60000)); expect(googleSheetsStatus).toHaveBeenCalledTimes(calls);
  });
});
