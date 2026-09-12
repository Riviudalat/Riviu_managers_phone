import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { GuiServiceSection } from "./GuiServiceSection";
import { guiServiceSave } from "../../api";
vi.mock("../../api", () => ({
  guiServiceStatus: vi.fn(async () => ({
    config: { enabled: true, baseUrl: "", model: "", maxRequests: 20 },
    running: false,
    providerReady: false,
    protocolVersion: 1,
  })),
  guiServiceSave: vi.fn(async () => undefined),
  guiServiceCheck: vi.fn(async () => "Sẵn sàng"),
  guiCompatibilityImport: vi.fn(),
  guiCompatibilityRollback: vi.fn(),
  guiDiagnosticsExport: vi.fn(),
}));
vi.mock("../../workspaceDraft", () => ({ useWorkspaceDraft: vi.fn() }));
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});
describe("GUI perception configuration", () => {
  it("shows provider availability separately from process state", async () => {
    render(<GuiServiceSection />);
    expect(await screen.findByText(/Chưa có khóa AI/)).toBeVisible();
    expect(screen.getByText(/Dịch vụ khởi động khi cần/)).toBeVisible();
    expect(
      screen.getByRole("button", { name: "Lưu cấu hình nhận diện" }),
    ).toBeDisabled();
  });
  it("rejects an invalid budget before saving", async () => {
    render(<GuiServiceSection />);
    const field = await screen.findByRole("spinbutton");
    await userEvent.clear(field);
    await userEvent.type(field, "0");
    await userEvent.click(
      screen.getByRole("button", { name: "Lưu cấu hình nhận diện" }),
    );
    expect(
      screen.getByText(/Giới hạn request phải từ 1 đến 100/),
    ).toBeVisible();
    expect(guiServiceSave).not.toHaveBeenCalled();
  });
});
