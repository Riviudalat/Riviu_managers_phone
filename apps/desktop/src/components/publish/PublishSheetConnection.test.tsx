import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { googleSheetsStatus, publishSheetGetConfig, publishSheetCheck } from "../../api";
import { PublishSheetConnection } from "./PublishSheetConnection";
vi.mock("../../api", () => ({ googleSheetsStatus: vi.fn(), publishSheetGetConfig: vi.fn(), publishSheetCheck: vi.fn() }));
beforeEach(() => { vi.resetAllMocks(); vi.mocked(googleSheetsStatus).mockResolvedValue({ configured: true, connected: false, active: false, clientId: "id", pickerConfigured: true, phase: "idle" }); vi.mocked(publishSheetGetConfig).mockResolvedValue({ webhookUrl: "old-exec", hasToken: true, provider: "appsScript", sheetUrl: "https://docs.google.com/spreadsheets/d/old/edit#gid=0" }); });
afterEach(cleanup);
it("renders the Google-only compact row and restores only the saved URL", async () => {
  const ready = vi.fn(); render(<PublishSheetConnection onReadyChange={ready} />);
  await waitFor(() => expect(screen.getByRole("textbox", { name: "Link Google Sheet" })).toHaveValue("https://docs.google.com/spreadsheets/d/old/edit#gid=0"));
  expect(screen.getAllByRole("button").map(b => b.textContent)).toEqual(["Đăng nhập Google", "Kiểm tra kết nối"]);
  expect(screen.queryByText(/Apps Script|nâng cao|Cấu hình/)).toBeNull(); expect(publishSheetCheck).not.toHaveBeenCalled(); expect(ready).toHaveBeenLastCalledWith(false);
});
