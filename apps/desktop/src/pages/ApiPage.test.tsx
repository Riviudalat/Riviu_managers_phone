import { StrictMode } from "react";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ApiPage } from "./ApiPage";

const loadDocs = vi.hoisted(() => vi.fn());

vi.mock("../api", () => ({ apiDocs: loadDocs, localApiStatus: vi.fn(async () => ({ running: false, activePort: null, restartRequired: false, lastError: null })) }));

beforeEach(() => {
  loadDocs.mockReset();
});

describe("ApiPage load states", () => {
  it("searches localized groups and individual runtime commands", async () => {
    loadDocs.mockResolvedValue("## Devices\n- list_devices / device_health\n## Operations\n- operation_get_run");
    render(<ApiPage />);
    await screen.findByText("Thiết bị");
    await userEvent.type(screen.getByRole("searchbox", { name: "Tìm lệnh API" }), "device_health");
    expect(screen.getByText("device_health")).toBeVisible();
    expect(screen.queryByText("operation_get_run")).toBeNull();
    expect(screen.queryByText("list_devices")).toBeNull();
    await userEvent.clear(screen.getByRole("searchbox", { name: "Tìm lệnh API" }));
    await userEvent.type(screen.getByRole("searchbox", { name: "Tìm lệnh API" }), "Thiết bị");
    expect(screen.getByText("list_devices")).toBeVisible();
  });

  it("shows loading, then the documentation without a duplicate page heading", async () => {
    loadDocs.mockResolvedValue("GET /health\n200 OK");

    render(<ApiPage />);

    expect(screen.getByRole("status")).toHaveTextContent("Đang tải tài liệu API");
    expect(screen.queryByRole("heading", { level: 2 })).toBeNull();
    expect(await screen.findByText(/GET \/health/)).toBeInTheDocument();
    expect(screen.queryByRole("searchbox")).toBeNull();
  });

  it("shows an inline error and retries the request", async () => {
    loadDocs.mockRejectedValueOnce(new Error("API chưa khởi động")).mockResolvedValueOnce("OK");

    render(<ApiPage />);

    expect(await screen.findByRole("alert")).toHaveTextContent("API chưa khởi động");
    await userEvent.click(screen.getByRole("button", { name: "Thử lại" }));

    await waitFor(() => expect(loadDocs).toHaveBeenCalledTimes(2));
    expect(await screen.findByText("OK")).toBeInTheDocument();
  });

  it("distinguishes an empty response from loading and offers a reload", async () => {
    loadDocs.mockResolvedValue("   \n");

    render(<ApiPage />);

    expect(await screen.findByText("Chưa có tài liệu API")).toBeInTheDocument();
    expect(screen.queryByText("Đang tải tài liệu API…")).toBeNull();
    expect(screen.getByRole("button", { name: "Tải lại" })).toBeEnabled();
  });

  it("keeps the newest documentation when StrictMode responses arrive out of order", async () => {
    let resolveFirst!: (value: string) => void;
    let resolveSecond!: (value: string) => void;
    loadDocs
      .mockReturnValueOnce(new Promise((resolve) => { resolveFirst = resolve; }))
      .mockReturnValueOnce(new Promise((resolve) => { resolveSecond = resolve; }));

    render(<StrictMode><ApiPage /></StrictMode>);
    await waitFor(() => expect(loadDocs).toHaveBeenCalledTimes(2));
    resolveSecond("NEW /v2");
    expect(await screen.findByText("NEW /v2")).toBeInTheDocument();
    resolveFirst("OLD /v1");
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(screen.getByText("NEW /v2")).toBeInTheDocument();
    expect(screen.queryByText("OLD /v1")).toBeNull();
  });
});
