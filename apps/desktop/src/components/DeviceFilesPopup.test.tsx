import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DeviceFilesPopup } from "./DeviceFilesPopup";
import { deviceDeletePath, deviceListDir, devicePullPath } from "../api";
import { requestConfirm } from "../confirmStore";
import { pickDirectory } from "../pickFile";
import type { DeviceFileEntry, DeviceInfo } from "../types";

vi.mock("../api", () => ({
  deviceListDir: vi.fn(),
  devicePullPath: vi.fn(),
  devicePushFile: vi.fn(),
  deviceDeletePath: vi.fn(),
}));
vi.mock("../confirmStore", () => ({ requestConfirm: vi.fn() }));
vi.mock("../pickFile", () => ({ pickDirectory: vi.fn(), pickFile: vi.fn() }));
// `describeError` is the real one on purpose: it is what turns a command's `{code, message}`
// rejection into a sentence, and mocking it away would let "[object Object]" back in unseen.
vi.mock("../toastStore", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../toastStore")>()),
  pushToast: vi.fn(),
  toastError: vi.fn(),
}));

const listMock = vi.mocked(deviceListDir);
const pullMock = vi.mocked(devicePullPath);
const deleteMock = vi.mocked(deviceDeletePath);
const confirmMock = vi.mocked(requestConfirm);
const dirMock = vi.mocked(pickDirectory);

const REDMI: DeviceInfo = {
  udid: "10969614",
  name: "Redmi 12C",
  model: "23021RAAEG",
  platform: "android",
  osVersion: "15",
  connection: "usb",
  status: "ready",
  wdaReady: true,
  tileStreamState: "sampling",
};

/** The measured listing of `/sdcard/Download` on this fleet, trimmed to four rows. */
const DOWNLOAD: DeviceFileEntry[] = [
  { name: "CV prototype.pdf", kind: "file", size: 138_078, modified: "2025-11-25 08:49", linkTarget: null },
  { name: "Browser", kind: "directory", size: 3452, modified: "2024-11-02 23:18", linkTarget: null },
  { name: ".tistore", kind: "file", size: 16_132, modified: "2025-01-19 12:24", linkTarget: null },
];

beforeEach(() => {
  listMock.mockReset();
  pullMock.mockReset();
  deleteMock.mockReset();
  confirmMock.mockReset();
  dirMock.mockReset();
});
afterEach(cleanup);

// Path arithmetic, sorting and sizes are proven in deviceFiles.test.ts without a DOM. What
// is here is the behaviour that only exists once the two are wired: which path is asked
// for, what a refusal looks like, and that a delete is confirmed by name and drops the
// selection afterwards.
describe("DeviceFilesPopup", () => {
  it("opens on the phone's own storage and lists it, folders first", async () => {
    listMock.mockResolvedValue(DOWNLOAD);
    render(<DeviceFilesPopup device={REDMI} onClose={vi.fn()} />);

    await waitFor(() => expect(listMock).toHaveBeenCalledWith("10969614", "/sdcard"));
    const names = screen.getAllByRole("checkbox").map((box) => box.getAttribute("aria-label"));
    expect(names).toEqual(["Chọn Browser", "Chọn .tistore", "Chọn CV prototype.pdf"]);
  });

  it("steps into a folder by asking for that path", async () => {
    listMock.mockResolvedValue(DOWNLOAD);
    render(<DeviceFilesPopup device={REDMI} onClose={vi.fn()} />);
    await waitFor(() => expect(screen.getByText(/Browser/)).toBeTruthy());

    await userEvent.click(screen.getByRole("button", { name: /Browser/ }));

    await waitFor(() => expect(listMock).toHaveBeenCalledWith("10969614", "/sdcard/Browser"));
  });

  /**
   * The distinction a file manager must not blur: a phone that refuses `ls` and a folder
   * that is empty are different facts, and only one of them is the operator's fault.
   */
  it("shows the phone's refusal rather than an empty folder", async () => {
    listMock.mockRejectedValue(new Error("không đọc được /sdcard/Android/data: Permission denied"));
    render(<DeviceFilesPopup device={REDMI} onClose={vi.fn()} />);

    await waitFor(() => expect(screen.getByRole("alert")).toBeTruthy());
    expect(screen.getByRole("alert").textContent).toContain("Permission denied");
    expect(screen.queryByText("Thư mục này rỗng.")).toBeNull();
  });

  it("says a folder is empty when the phone answered with nothing", async () => {
    listMock.mockResolvedValue([]);
    render(<DeviceFilesPopup device={REDMI} onClose={vi.fn()} />);

    await waitFor(() => expect(screen.getByText("Thư mục này rỗng.")).toBeTruthy());
  });

  it("confirms a delete by naming the files, then re-reads the folder", async () => {
    listMock.mockResolvedValue(DOWNLOAD);
    confirmMock.mockResolvedValue(true);
    deleteMock.mockResolvedValue(undefined);
    render(<DeviceFilesPopup device={REDMI} onClose={vi.fn()} />);
    await waitFor(() => expect(screen.getByLabelText("Chọn .tistore")).toBeTruthy());

    await userEvent.click(screen.getByLabelText("Chọn .tistore"));
    await userEvent.click(screen.getByRole("button", { name: "Xoá" }));

    await waitFor(() => expect(confirmMock).toHaveBeenCalled());
    expect(confirmMock.mock.calls[0][0].message).toContain(".tistore");
    await waitFor(() =>
      expect(deleteMock).toHaveBeenCalledWith("10969614", "/sdcard/.tistore"),
    );
    // Re-read, so a listing showing a file that is gone cannot be acted on again.
    await waitFor(() => expect(listMock).toHaveBeenCalledTimes(2));
    expect(screen.getByText("Chưa chọn mục nào")).toBeTruthy();
  });

  it("deletes nothing when the confirm is declined", async () => {
    listMock.mockResolvedValue(DOWNLOAD);
    confirmMock.mockResolvedValue(false);
    render(<DeviceFilesPopup device={REDMI} onClose={vi.fn()} />);
    await waitFor(() => expect(screen.getByLabelText("Chọn .tistore")).toBeTruthy());

    await userEvent.click(screen.getByLabelText("Chọn .tistore"));
    await userEvent.click(screen.getByRole("button", { name: "Xoá" }));

    await waitFor(() => expect(confirmMock).toHaveBeenCalled());
    expect(deleteMock).not.toHaveBeenCalled();
  });

  it("pulls every picked row and keeps going past one that fails", async () => {
    listMock.mockResolvedValue(DOWNLOAD);
    dirMock.mockResolvedValue("D:\\export");
    pullMock
      .mockRejectedValueOnce(new Error("adb pull thất bại"))
      .mockResolvedValueOnce("D:\\export\\CV prototype.pdf");
    render(<DeviceFilesPopup device={REDMI} onClose={vi.fn()} />);
    await waitFor(() => expect(screen.getByLabelText("Chọn .tistore")).toBeTruthy());

    await userEvent.click(screen.getByLabelText("Chọn .tistore"));
    await userEvent.click(screen.getByLabelText("Chọn CV prototype.pdf"));
    await userEvent.click(screen.getByRole("button", { name: "Lấy về máy tính" }));

    await waitFor(() => expect(pullMock).toHaveBeenCalledTimes(2));
    expect(pullMock.mock.calls[1]).toEqual([
      "10969614",
      "/sdcard/CV prototype.pdf",
      "D:\\export",
    ]);
  });

  it("drops the selection when the listing changes underneath it", async () => {
    listMock.mockResolvedValue(DOWNLOAD);
    render(<DeviceFilesPopup device={REDMI} onClose={vi.fn()} />);
    await waitFor(() => expect(screen.getByLabelText("Chọn .tistore")).toBeTruthy());
    await userEvent.click(screen.getByLabelText("Chọn .tistore"));
    expect(screen.getByText("Đã chọn 1 mục")).toBeTruthy();

    await userEvent.click(screen.getByRole("button", { name: /Browser/ }));

    // Same names live in twenty folders; a selection that survives a navigation is how a
    // delete lands on the wrong file.
    await waitFor(() => expect(screen.getByText("Chưa chọn mục nào")).toBeTruthy());
  });

  /**
   * Shortcuts and breadcrumbs can only reach what is already on screen. This is how an
   * operator gets to `/data/local/tmp` — the thing a file browser exists for.
   */
  it("goes to a typed path on Enter", async () => {
    listMock.mockResolvedValue(DOWNLOAD);
    render(<DeviceFilesPopup device={REDMI} onClose={vi.fn()} />);
    await waitFor(() => expect(listMock).toHaveBeenCalledWith("10969614", "/sdcard"));

    const box = screen.getByLabelText("Đường dẫn");
    await userEvent.clear(box);
    await userEvent.type(box, "/data/local/tmp{Enter}");

    await waitFor(() =>
      expect(listMock).toHaveBeenCalledWith("10969614", "/data/local/tmp"),
    );
  });

  it("refuses a relative path with a reason instead of asking the phone", async () => {
    listMock.mockResolvedValue(DOWNLOAD);
    render(<DeviceFilesPopup device={REDMI} onClose={vi.fn()} />);
    await waitFor(() => expect(listMock).toHaveBeenCalledTimes(1));

    const box = screen.getByLabelText("Đường dẫn");
    await userEvent.clear(box);
    await userEvent.type(box, "sdcard/Download{Enter}");

    expect(screen.getByRole("alert").textContent).toContain("bắt đầu bằng /");
    expect(listMock).toHaveBeenCalledTimes(1);
  });

  it("keeps the path box in step with where the browser actually is", async () => {
    listMock.mockResolvedValue(DOWNLOAD);
    render(<DeviceFilesPopup device={REDMI} onClose={vi.fn()} />);
    await waitFor(() => expect(screen.getByText(/Browser/)).toBeTruthy());

    await userEvent.click(screen.getByRole("button", { name: /Browser/ }));

    await waitFor(() =>
      expect((screen.getByLabelText("Đường dẫn") as HTMLInputElement).value).toBe(
        "/sdcard/Browser",
      ),
    );
  });
});
