import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { googleSheetsStatus, publishSheetGetConfig, publishSheetCheck } from "../../api";
import { PublishSheetConnection } from "./PublishSheetConnection";
vi.mock("../../api", () => ({ googleSheetsStatus: vi.fn(), publishSheetGetConfig: vi.fn(), publishSheetCheck: vi.fn() }));
beforeEach(() => { vi.resetAllMocks(); vi.mocked(googleSheetsStatus).mockResolvedValue({ configured: true, connected: false, active: false, clientId: "id", pickerConfigured: true, phase: "idle" }); vi.mocked(publishSheetGetConfig).mockResolvedValue({ webhookUrl: "", hasToken: false }); });
afterEach(cleanup);
it("late saved URL never overwrites an operator edit", async () => {
  let resolve!: (value: Awaited<ReturnType<typeof publishSheetGetConfig>>) => void;
  vi.mocked(publishSheetGetConfig).mockReturnValue(new Promise(yes => { resolve = yes; })); render(<PublishSheetConnection />);
  fireEvent.change(screen.getByRole("textbox", { name: "Link Google Sheet" }), { target: { value: "https://docs.google.com/spreadsheets/d/new/edit#gid=3" } });
  await act(async () => resolve({ webhookUrl: "old", hasToken: true, sheetUrl: "https://docs.google.com/spreadsheets/d/old/edit" }));
  expect(screen.getByRole("textbox", { name: "Link Google Sheet" })).toHaveValue("https://docs.google.com/spreadsheets/d/new/edit#gid=3");
  await waitFor(() => expect(screen.getByRole("button", { name: "Đăng nhập Google" })).toBeEnabled()); expect(publishSheetCheck).not.toHaveBeenCalled();
});
