import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { TypeSafeSettingsCard } from "./TypeSafeSettingsCard";
import * as api from "../../api";
vi.mock("../../api", () => ({
  typesafeGetSettings: vi.fn(), typesafeUpdateSettings: vi.fn(),
  typesafeUpdateCredential: vi.fn(), typesafeCheckComment: vi.fn(),
}));
beforeEach(() => {
  vi.mocked(api.typesafeGetSettings).mockResolvedValue({ enabled: false, hasApiKey: false, revision: 4 });
});
afterEach(() => { cleanup(); vi.clearAllMocks(); });
it("does not test the old credential while an edited key is unsaved", async () => {
  vi.mocked(api.typesafeGetSettings).mockResolvedValue({ enabled: true, hasApiKey: true, revision: 4 });
  render(<TypeSafeSettingsCard />);
  await waitFor(() => expect(screen.getByRole("button", { name: "Kiểm tra bằng mẫu chữ" })).toBeEnabled());
  fireEvent.change(screen.getByLabelText("Khóa TypeSafe"), { target: { value: "new-key" } });
  expect(screen.getByRole("button", { name: "Kiểm tra bằng mẫu chữ" })).toBeDisabled();
  expect(screen.getByText(/Lưu khóa mới trước khi kiểm tra/)).toBeVisible();
  expect(api.typesafeCheckComment).not.toHaveBeenCalled();
});
it("reads status without an automatic inference call and saves the credential separately", async () => {
  render(<TypeSafeSettingsCard />);
  await waitFor(() => expect(screen.getByRole("checkbox")).toBeEnabled());
  expect(api.typesafeCheckComment).not.toHaveBeenCalled();
  vi.mocked(api.typesafeUpdateCredential).mockResolvedValue({ enabled: false, hasApiKey: true, revision: 4 });
  fireEvent.change(screen.getByLabelText("Khóa TypeSafe"), { target: { value: "fixture-secret" } });
  fireEvent.click(screen.getByRole("button", { name: "Lưu khóa TypeSafe" }));
  await screen.findByText("Đã lưu khóa TypeSafe.");
  expect(api.typesafeUpdateCredential).toHaveBeenCalledWith("fixture-secret");
  expect(api.typesafeUpdateSettings).not.toHaveBeenCalled();
  expect(screen.getByLabelText("Khóa TypeSafe")).toHaveValue("");
});
it("sends the current revision and refreshes after a conflicting settings save", async () => {
  render(<TypeSafeSettingsCard />);
  await waitFor(() => expect(screen.getByRole("checkbox")).toBeEnabled());
  vi.mocked(api.typesafeUpdateSettings).mockRejectedValue(new Error("TypeSafeSettingsConflict"));
  vi.mocked(api.typesafeGetSettings).mockResolvedValue({ enabled: true, hasApiKey: true, revision: 5 });
  fireEvent.click(screen.getByRole("checkbox"));
  await screen.findByText(/TypeSafeSettingsConflict/);
  expect(api.typesafeUpdateSettings).toHaveBeenCalledWith(true, 4);
  await waitFor(() => expect(screen.getByRole("checkbox")).toBeChecked());
});
