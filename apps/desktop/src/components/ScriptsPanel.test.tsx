import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ScriptsPanel } from "./ScriptsPanel";

const { exampleScript, listScripts } = vi.hoisted(() => ({
  exampleScript: vi.fn(async () => '{"steps":[]}'),
  listScripts: vi.fn(),
}));

vi.mock("../api", () => ({
  exampleScript,
  listScripts,
  saveScript: vi.fn(async () => undefined),
}));

afterEach(cleanup);
beforeEach(() => {
  vi.clearAllMocks();
  listScripts.mockReset();
  listScripts.mockResolvedValue([] as [string, string][]);
});

describe("ScriptsPanel Save", () => {
  it("keeps the legacy-script scope under the broader Flow topbar", () => {
    render(<ScriptsPanel onUseInJobs={() => undefined} />);

    expect(screen.getByRole("heading", { level: 2, name: "Kịch bản" })).toBeVisible();
  });

  it("starts blank and only loads the example after an explicit action", async () => {
    render(<ScriptsPanel onUseInJobs={() => undefined} />);

    expect(screen.getByRole("textbox", { name: "Tên" })).toHaveValue("");
    expect(screen.getByRole("textbox", { name: "Kịch bản JSON" })).toHaveValue("");
    expect(exampleScript).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Tải mẫu" }));

    await waitFor(() => expect(exampleScript).toHaveBeenCalledTimes(1));
    expect(screen.getByRole("textbox", { name: "Kịch bản JSON" })).toHaveValue('{"steps":[]}');
  });

  it("says why a script was not saved instead of leaving the panel silent", async () => {
    // `await saveScript(...)` had no guard, and the backend parses a script before storing
    // it — so this rejects on exactly the input an operator most needs told about, a
    // script with a syntax error. The rejection went nowhere: no confirmation, no reason,
    // and the panel still showing text that had not been stored.
    const api = await import("../api");
    vi.mocked(api.saveScript).mockRejectedValueOnce(
      new Error("expected `steps` to be an array at line 3"),
    );
    render(<ScriptsPanel onUseInJobs={() => undefined} />);

    fireEvent.click(screen.getByRole("button", { name: "Lưu" }));

    expect(
      await screen.findByText(/Không lưu được: expected `steps` to be an array at line 3/),
    ).toBeInTheDocument();
  });

  it("confirms a save that worked, and reloads the saved list", async () => {
    const api = await import("../api");
    render(<ScriptsPanel onUseInJobs={() => undefined} />);

    fireEvent.click(screen.getByRole("button", { name: "Lưu" }));

    expect(await screen.findByText("Đã lưu")).toBeInTheDocument();
    await waitFor(() => expect(api.listScripts).toHaveBeenCalledTimes(2));
  });

  it("shows loading, a retryable read error, then the saved data", async () => {
    const api = await import("../api");
    let rejectFirst!: (reason: unknown) => void;
    vi.mocked(api.listScripts)
      .mockImplementationOnce(
        () => new Promise((_, reject) => {
          rejectFirst = reject;
        }),
      )
      .mockResolvedValueOnce([["daily", '{"steps":[]}']]);

    render(<ScriptsPanel onUseInJobs={() => undefined} />);
    expect(screen.getByRole("status")).toHaveTextContent("Đang tải kịch bản đã lưu");

    rejectFirst(new Error("database is locked"));
    expect(await screen.findByRole("alert")).toHaveTextContent("database is locked");

    fireEvent.click(screen.getByRole("button", { name: "Thử lại danh sách" }));
    expect(await screen.findByRole("button", { name: "daily" })).toBeInTheDocument();
    expect(api.listScripts).toHaveBeenCalledTimes(2);
  });

  it("distinguishes a successful empty list from loading and failure", async () => {
    render(<ScriptsPanel onUseInJobs={() => undefined} />);

    expect(await screen.findByText("Chưa có kịch bản đã lưu")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).toBeNull();
  });
});
