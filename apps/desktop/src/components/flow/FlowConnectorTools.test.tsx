import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { FlowConnectorTools } from "./FlowConnectorTools";

const mocks = vi.hoisted(() => ({ info: vi.fn(), save: vi.fn(), upload: vi.fn() }));
vi.mock("../../api", () => ({ flowConnectorInfo: mocks.info, flowConnectorSaveSecret: mocks.save, flowConnectorImportFile: mocks.upload }));
beforeEach(() => {
  vi.clearAllMocks();
  mocks.info.mockResolvedValue({ root: "C:/fixture/flow-data", credentialNames: [], sheetConfigured: true });
  mocks.save.mockResolvedValue(undefined); mocks.upload.mockResolvedValue(undefined);
});
afterEach(cleanup);

it("loads reference names only and saves a token once while clearing the password", async () => {
  render(<FlowConnectorTools />);
  await screen.findByText("C:/fixture/flow-data");
  expect(mocks.save).not.toHaveBeenCalled(); expect(mocks.upload).not.toHaveBeenCalled();
  fireEvent.click(screen.getByText("Kết nối dữ liệu: tệp, HTTP và Google Sheet"));
  fireEvent.change(screen.getByLabelText("Tên tham chiếu"), { target: { value: "partner_api" } });
  fireEvent.change(screen.getByLabelText("Token"), { target: { value: "private-value" } });
  fireEvent.click(screen.getByRole("button", { name: "Lưu token" }));
  await screen.findByRole("status");
  expect(mocks.save).toHaveBeenCalledExactlyOnceWith("partner_api", "private-value");
  expect(screen.getByLabelText("Token")).toHaveValue("");
  expect(document.body.textContent).not.toContain("private-value");
});

it("imports one bounded file and does not upload an oversized file", async () => {
  render(<FlowConnectorTools />);
  await screen.findByText("C:/fixture/flow-data");
  const field = screen.getByLabelText("Nhập tệp UTF-8 (tối đa 4.096 ký tự)");
  fireEvent.change(field, { target: { files: [{ name: "rows.csv", size: 12, text: async () => "a,b\n1,2" }] } });
  await waitFor(() => expect(mocks.upload).toHaveBeenCalledExactlyOnceWith("rows.csv", "a,b\n1,2"));
  await screen.findByRole("status");
  fireEvent.change(field, { target: { files: [{ name: "large.txt", size: 20000, text: async () => "x" }] } });
  await screen.findByRole("alert");
  expect(mocks.upload).toHaveBeenCalledTimes(1);
});

it("surfaces credential-store errors without success or duplicate requests", async () => {
  mocks.save.mockRejectedValue(new Error("Credential store offline"));
  render(<FlowConnectorTools />);
  await screen.findByText("C:/fixture/flow-data");
  fireEvent.click(screen.getByText("Kết nối dữ liệu: tệp, HTTP và Google Sheet"));
  fireEvent.change(screen.getByLabelText("Tên tham chiếu"), { target: { value: "partner_api" } });
  fireEvent.change(screen.getByLabelText("Token"), { target: { value: "private" } });
  fireEvent.click(screen.getByRole("button", { name: "Lưu token" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Credential store offline");
  expect(screen.queryByRole("status")).toBeNull();
  expect(mocks.save).toHaveBeenCalledTimes(1);
});
