import { StrictMode } from "react";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ApiRuntimeStatus } from "./ApiRuntimeStatus";
import type { LocalApiStatus } from "../../api";
const read = vi.hoisted(() => vi.fn());
vi.mock("../../api", () => ({ localApiStatus: read }));
const off: LocalApiStatus = { configuredEnabled:false, configuredPort:3200, running:false,activePort:null,restartRequired:false,lastError:null };
beforeEach(() => read.mockReset());

describe("ApiRuntimeStatus", () => {
  it("shows the actual active port, not the newly configured port", async () => {
    read.mockResolvedValue({...off,configuredEnabled:true,configuredPort:3300,running:true,activePort:3200,restartRequired:true});
    render(<ApiRuntimeStatus />);
    expect(await screen.findByText("Đang lắng nghe")).toBeVisible();
    expect(screen.getByText("127.0.0.1:3200")).toBeVisible();
    expect(screen.queryByText("127.0.0.1:3300")).toBeNull();
    expect(screen.getByText("Cấu hình mới có hiệu lực sau khi khởi động lại ứng dụng.")).toBeVisible();
  });
  it("reports off only for an observed stopped listener", async () => {
    read.mockResolvedValue(off);
    render(<ApiRuntimeStatus />);
    expect(await screen.findByText("Đã tắt")).toBeVisible();
    expect(screen.queryByText(/127\.0\.0\.1:/)).toBeNull();
  });
  it("does not label an unknown runtime as stopped", async () => {
    read.mockResolvedValue({...off,running:null});
    render(<ApiRuntimeStatus />);
    expect(await screen.findByText("Chưa xác định")).toBeVisible();
    expect(screen.queryByText("Đã tắt")).toBeNull();
  });
  it("distinguishes listener failure from a status-read failure and retries", async () => {
    read.mockRejectedValueOnce(new Error("status unavailable")).mockResolvedValueOnce({...off,lastError:"port already in use"});
    render(<ApiRuntimeStatus />);
    expect(await screen.findByRole("alert")).toHaveTextContent("Chưa xác định kết nối API: status unavailable");
    await userEvent.click(screen.getByRole("button", {name:"Kiểm lại kết nối API"}));
    expect(await screen.findByText("Kết nối gặp lỗi")).toBeVisible();
    await userEvent.click(screen.getByText("Chi tiết lỗi"));
    expect(screen.getByText("port already in use")).toBeVisible();
  });
  it("ignores an older StrictMode response after the latest status resolved", async () => {
    let resolve!: (value: LocalApiStatus) => void;
    read.mockReturnValueOnce(new Promise((done) => {resolve=done;})).mockResolvedValueOnce(off);
    render(<StrictMode><ApiRuntimeStatus /></StrictMode>);
    await waitFor(() => expect(read).toHaveBeenCalledTimes(2));
    expect(await screen.findByText("Đã tắt")).toBeVisible();
    await act(async () => resolve({...off,running:true,activePort:3200}));
    expect(screen.queryByText("Đang lắng nghe")).toBeNull();
  });
});
