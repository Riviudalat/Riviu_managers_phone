import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { googleSheetsCancel, googleSheetsConfigure, googleSheetsConnect, googleSheetsLogin, googleSheetsPickFile, googleSheetsStatus, publishSheetCheck, publishSheetGetConfig } from "../../api";
import type { GoogleSheetsStatus, PublishSheetCheckResult } from "../../types";
import { GoogleSheetConnection } from "./GoogleSheetConnection";
import { parseGoogleSheetUrl } from "./googleSheetUrl";
import { requestConfirm } from "../../confirmStore";
vi.mock("../../confirmStore", () => ({ requestConfirm: vi.fn(async () => false) }));
vi.mock("../../api", () => ({ googleSheetsCancel: vi.fn(), googleSheetsConfigure: vi.fn(), googleSheetsConnect: vi.fn(), googleSheetsLogin: vi.fn(), googleSheetsPickFile: vi.fn(), googleSheetsStatus: vi.fn(), publishSheetCheck: vi.fn(), publishSheetGetConfig: vi.fn() }));
const url = "https://docs.google.com/spreadsheets/d/file-a/edit#gid=7";
const status = (patch: Partial<GoogleSheetsStatus> = {}): GoogleSheetsStatus => ({ configured: true, connected: true, active: false, accountId: "account-a", email: "operator@example.test", clientId: "id", pickerConfigured: true, phase: "idle", ...patch });
const active = (patch: Partial<GoogleSheetsStatus> = {}) => status({ active: true, selectedFileId: "file-a", writerId: "writer-a", sheetUrl: url, ...patch });
const checked = (patch: Partial<PublishSheetCheckResult> = {}): PublishSheetCheckResult => ({ sheetUrl: url, spreadsheetId: "file-a", sheetGid: 7, readable: true, connectionVerified: true, reportingReady: true, layout: "internal", columns: [], message: "Đã xác minh", ...patch });
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>(yes => { resolve = yes; }); return { promise, resolve }; }
beforeEach(() => { vi.resetAllMocks(); vi.mocked(googleSheetsStatus).mockResolvedValue(status()); vi.mocked(publishSheetGetConfig).mockResolvedValue({ webhookUrl: "", hasToken: false }); vi.mocked(publishSheetCheck).mockResolvedValue(checked()); vi.mocked(googleSheetsConnect).mockResolvedValue(checked()); vi.mocked(googleSheetsLogin).mockResolvedValue(status()); vi.mocked(googleSheetsCancel).mockResolvedValue(status()); vi.mocked(googleSheetsPickFile).mockResolvedValue(status({ selectedFileId: "file-a" })); });
afterEach(() => { cleanup(); vi.useRealTimers(); });
async function editUrl(value = url) { await waitFor(() => expect(screen.getByRole("button", { name: "Đăng nhập Google" })).toBeEnabled()); fireEvent.change(screen.getByRole("textbox", { name: "Link Google Sheet" }), { target: { value } }); }

describe("compact Google Sheet connection", () => {
  it("connects the saved URL automatically after OAuth without opening Picker", async () => {
    vi.mocked(googleSheetsStatus).mockResolvedValue(status({ connected: false, pickerConfigured: false, sheetUrl: url }));
    vi.mocked(googleSheetsLogin).mockResolvedValue(status({ pickerConfigured: false, sheetUrl: url }));
    const ready = vi.fn(); render(<GoogleSheetConnection onReadyChange={ready} />);
    await waitFor(() => expect(screen.getByRole("button", { name: "Đăng nhập Google" })).toBeEnabled());
    vi.mocked(googleSheetsStatus).mockResolvedValueOnce(status({ connected: false, pickerConfigured: false, sheetUrl: url })).mockResolvedValue(active({ pickerConfigured: false }));
    fireEvent.click(screen.getByRole("button", { name: "Đăng nhập Google" }));
    await waitFor(() => expect(ready).toHaveBeenLastCalledWith(true));
    expect(googleSheetsConnect).toHaveBeenCalledExactlyOnceWith("file-a", 7, true, false);
    expect(googleSheetsPickFile).not.toHaveBeenCalled();
    expect(screen.queryByText("Thiết lập Google")).toBeNull();
  });
  it("connects a typed URL directly even if another file was previously picked", async () => {
    vi.mocked(googleSheetsStatus).mockResolvedValue(status({ selectedFileId: "other-file", pickerConfigured: false }));
    const ready = vi.fn(); render(<GoogleSheetConnection onReadyChange={ready} />); await editUrl();
    vi.mocked(googleSheetsStatus).mockResolvedValueOnce(status({ selectedFileId: "other-file", pickerConfigured: false })).mockResolvedValue(active({ pickerConfigured: false }));
    fireEvent.click(screen.getByRole("button", { name: "Kiểm tra kết nối" }));
    await waitFor(() => expect(ready).toHaveBeenLastCalledWith(true));
    expect(googleSheetsConnect).toHaveBeenCalledExactlyOnceWith("file-a", 7, true, false);
    expect(googleSheetsPickFile).not.toHaveBeenCalled();
  });
  it("requires explicit old-writer drain confirmation only for legacy upgrade", async () => {
    vi.mocked(googleSheetsStatus).mockResolvedValue(status({ selectedFileId: "file-a" }));
    vi.mocked(googleSheetsConnect).mockRejectedValueOnce({ code: "SharedSheetUpgradeRequired", message: "Cần nâng cấp tab đa máy" }).mockResolvedValue(checked());
    vi.mocked(requestConfirm).mockResolvedValue(true);
    const ready = vi.fn(); render(<GoogleSheetConnection onReadyChange={ready} />); await editUrl();
    vi.mocked(googleSheetsStatus).mockResolvedValueOnce(status()).mockResolvedValue(active());
    fireEvent.click(screen.getByRole("button", { name: "Kiểm tra kết nối" }));
    await waitFor(() => expect(ready).toHaveBeenLastCalledWith(true));
    expect(requestConfirm).toHaveBeenCalledWith(expect.objectContaining({ message: expect.stringContaining("bản app cũ") }));
    expect(vi.mocked(googleSheetsConnect).mock.calls).toEqual([["file-a", 7, true, false], ["file-a", 7, true, true]]);
    expect(googleSheetsPickFile).not.toHaveBeenCalled();
  });
  async function fillSetup() {
    fireEvent.click(await screen.findByText("Thiết lập Google"));
    fireEvent.change(screen.getByLabelText("OAuth Client ID (Desktop)"), { target: { value: " fixture.apps.googleusercontent.com " } });
    fireEvent.change(screen.getByLabelText("Client secret"), { target: { value: " app-secret " } });
    fireEvent.change(screen.getByLabelText("Google Picker API key"), { target: { value: " picker-key " } });
    fireEvent.change(screen.getByLabelText("Google Cloud project number"), { target: { value: "12345" } });
  }
  it("lets a clean PC save application config without starting a Google login or claiming readiness", async () => {
    vi.mocked(googleSheetsStatus).mockResolvedValue(status({ configured: false, connected: false, pickerConfigured: false, clientId: "" }));
    const save = deferred<GoogleSheetsStatus>(); vi.mocked(googleSheetsConfigure).mockReturnValue(save.promise);
    const ready = vi.fn(); render(<GoogleSheetConnection onReadyChange={ready} />); await fillSetup();
    const button = screen.getByRole("button", { name: "Lưu cấu hình Google" }); fireEvent.click(button); fireEvent.click(button);
    await waitFor(() => expect(googleSheetsConfigure).toHaveBeenCalledExactlyOnceWith({ clientId: "fixture.apps.googleusercontent.com", clientSecret: "app-secret", pickerApiKey: "picker-key", projectNumber: "12345" }));
    expect(screen.getByLabelText("Client secret")).toBeDisabled();
    await act(async () => save.resolve(status({ connected: false })));
    expect(screen.queryByText("Thiết lập Google")).toBeNull(); expect(ready).toHaveBeenLastCalledWith(false);
    expect(googleSheetsLogin).not.toHaveBeenCalled(); expect(googleSheetsConnect).not.toHaveBeenCalled();
  });
  it("keeps setup available and reports a credential-store write failure", async () => {
    vi.mocked(googleSheetsStatus).mockResolvedValue(status({ configured: false, connected: false, pickerConfigured: false }));
    vi.mocked(googleSheetsConfigure).mockRejectedValue(Error("Không lưu được cấu hình vào kho thông tin xác thực"));
    render(<GoogleSheetConnection />); await fillSetup(); fireEvent.click(screen.getByRole("button", { name: "Lưu cấu hình Google" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Không lưu được");
    expect(screen.getByRole("button", { name: "Lưu cấu hình Google" })).toBeEnabled(); expect(googleSheetsLogin).not.toHaveBeenCalled();
  });
  it("does not require Picker configuration when Google OAuth is configured", async () => {
    vi.mocked(googleSheetsStatus).mockResolvedValue(status({ pickerConfigured: false, clientId: "existing.apps.googleusercontent.com" }));
    render(<GoogleSheetConnection />); await editUrl();
    expect(screen.queryByText("Thiết lập Google")).toBeNull();
    vi.mocked(googleSheetsStatus).mockResolvedValueOnce(status({ pickerConfigured: false })).mockResolvedValue(active({ pickerConfigured: false }));
    fireEvent.click(screen.getByRole("button", { name: "Kiểm tra kết nối" }));
    await waitFor(() => expect(googleSheetsConnect).toHaveBeenCalledOnce());
    expect(googleSheetsPickFile).not.toHaveBeenCalled();
  });
  it("can save OAuth configuration without optional Picker credentials", async () => {
    vi.mocked(googleSheetsStatus).mockResolvedValue(status({ configured: false, connected: false, pickerConfigured: false, clientId: "" }));
    vi.mocked(googleSheetsConfigure).mockResolvedValue(status({ connected: false, pickerConfigured: false }));
    render(<GoogleSheetConnection />);
    fireEvent.click(await screen.findByText("Thiết lập Google"));
    fireEvent.change(screen.getByLabelText("OAuth Client ID (Desktop)"), { target: { value: "fixture.apps.googleusercontent.com" } });
    const save = screen.getByRole("button", { name: "Lưu cấu hình Google" });
    expect(save).toBeEnabled(); fireEvent.click(save);
    await waitFor(() => expect(googleSheetsConfigure).toHaveBeenCalledWith({ clientId: "fixture.apps.googleusercontent.com", clientSecret: undefined, pickerApiKey: "", projectNumber: "" }));
    expect(screen.queryByRole("alert")).toBeNull();
  });
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
    expect(googleSheetsConnect).toHaveBeenCalledExactlyOnceWith("file-a", 7, true, false); expect(googleSheetsPickFile).not.toHaveBeenCalled();
  });
  it("reports denied Google write access without opening Picker or choosing another file", async () => {
    vi.mocked(googleSheetsConnect).mockRejectedValue({ code: "OperationFailed", message: "Tài khoản không có quyền sửa bảng" });
    const ready = vi.fn(); render(<GoogleSheetConnection onReadyChange={ready} />); await editUrl();
    fireEvent.click(screen.getByRole("button", { name: "Kiểm tra kết nối" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("không có quyền sửa");
    expect(screen.getByRole("textbox", { name: "Link Google Sheet" })).toHaveValue(url);
    expect(googleSheetsPickFile).not.toHaveBeenCalled(); expect(ready).toHaveBeenLastCalledWith(false);
  });
  it("waits for OAuth callback then connects without another button", async () => {
    vi.useFakeTimers(); vi.mocked(googleSheetsLogin).mockResolvedValue(status({ phase: "authorizing" }));
    const ready = vi.fn(); render(<GoogleSheetConnection onReadyChange={ready} />); await act(async () => {});
    fireEvent.change(screen.getByRole("textbox", { name: "Link Google Sheet" }), { target: { value: url } });
    vi.mocked(googleSheetsStatus).mockResolvedValueOnce(status()).mockResolvedValueOnce(status()).mockResolvedValueOnce(active());
    fireEvent.click(screen.getByRole("button", { name: "Đăng nhập Google" })); await act(async () => {});
    expect(screen.getByRole("status")).toHaveTextContent("Hoàn tất đăng nhập");
    await act(async () => vi.advanceTimersByTimeAsync(1500));
    expect(googleSheetsConnect).toHaveBeenCalledExactlyOnceWith("file-a", 7, true, false); expect(ready).toHaveBeenLastCalledWith(true);
    expect(googleSheetsPickFile).not.toHaveBeenCalled();
  });
  it("does not approve a legacy upgrade after the URL changes during confirmation", async () => {
    const confirmation = deferred<boolean>(); vi.mocked(requestConfirm).mockReturnValue(confirmation.promise);
    vi.mocked(googleSheetsConnect).mockRejectedValueOnce({ code: "SharedSheetUpgradeRequired", message: "Nâng cấp" });
    const ready = vi.fn(); render(<GoogleSheetConnection onReadyChange={ready} />); await editUrl();
    fireEvent.click(screen.getByRole("button", { name: "Kiểm tra kết nối" }));
    await waitFor(() => expect(requestConfirm).toHaveBeenCalledOnce());
    fireEvent.change(screen.getByRole("textbox", { name: "Link Google Sheet" }), { target: { value: url.replace("file-a", "changed") } });
    await act(async () => confirmation.resolve(true));
    expect(googleSheetsConnect).toHaveBeenCalledExactlyOnceWith("file-a", 7, true, false);
    expect(ready).toHaveBeenLastCalledWith(false);
  });
  it("preserves a false readiness read instead of preparing the active target implicitly", async () => {
    vi.mocked(googleSheetsStatus).mockResolvedValue(active());
    vi.mocked(publishSheetCheck).mockResolvedValue(checked({ connectionVerified: false, reportingReady: false, message: "Đọc được nhưng chưa đủ quyền ghi" }));
    const ready = vi.fn(); render(<GoogleSheetConnection onReadyChange={ready} />);
    await waitFor(() => expect(screen.getByRole("button", { name: "Kiểm tra kết nối" })).toBeEnabled());
    fireEvent.click(screen.getByRole("button", { name: "Kiểm tra kết nối" }));
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("chưa đủ quyền ghi"));
    expect(googleSheetsConnect).not.toHaveBeenCalled(); expect(ready).toHaveBeenLastCalledWith(false);
  });
  it("invalidates a late read after editing, even when the URL changes back", async () => {
    vi.mocked(googleSheetsStatus).mockResolvedValue(active()); const read = deferred<PublishSheetCheckResult>(); vi.mocked(publishSheetCheck).mockReturnValue(read.promise);
    const ready = vi.fn(); render(<GoogleSheetConnection onReadyChange={ready} />); await waitFor(() => expect(publishSheetCheck).toHaveBeenCalled());
    const input = screen.getByRole("textbox", { name: "Link Google Sheet" }); fireEvent.change(input, { target: { value: url + "x" } }); fireEvent.change(input, { target: { value: url } });
    await act(async () => read.resolve(checked())); expect(ready).toHaveBeenLastCalledWith(false); expect(screen.getByRole("button", { name: "Kiểm tra kết nối" })).toBeEnabled();
  });
  it("does not auto-connect a changed URL when OAuth finishes", async () => {
    const login = deferred<GoogleSheetsStatus>(); vi.mocked(googleSheetsLogin).mockReturnValue(login.promise);
    const ready = vi.fn(); render(<GoogleSheetConnection onReadyChange={ready} />); await editUrl();
    fireEvent.click(screen.getByRole("button", { name: "Đăng nhập Google" }));
    await waitFor(() => expect(googleSheetsLogin).toHaveBeenCalledOnce());
    fireEvent.change(screen.getByRole("textbox", { name: "Link Google Sheet" }), { target: { value: url.replace("file-a", "file-b") } });
    await act(async () => login.resolve(active()));
    expect(googleSheetsConnect).not.toHaveBeenCalled(); expect(ready).toHaveBeenLastCalledWith(false);
  });
  it("does not migrate when the operator declines and clears staged OAuth", async () => {
    vi.mocked(googleSheetsConnect).mockRejectedValue({ code: "SharedSheetUpgradeRequired", message: "Cần nâng cấp" });
    vi.mocked(requestConfirm).mockResolvedValue(false);
    const ready = vi.fn(); render(<GoogleSheetConnection onReadyChange={ready} />); await editUrl();
    fireEvent.click(screen.getByRole("button", { name: "Kiểm tra kết nối" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Chưa nâng cấp tab");
    expect(googleSheetsConnect).toHaveBeenCalledExactlyOnceWith("file-a", 7, true, false);
    expect(googleSheetsCancel).toHaveBeenCalledOnce(); expect(ready).toHaveBeenLastCalledWith(false);
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
