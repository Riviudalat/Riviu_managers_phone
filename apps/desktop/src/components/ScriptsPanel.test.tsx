import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ScriptsPanel } from "./ScriptsPanel";

vi.mock("../api", () => ({
  exampleScript: vi.fn(async () => '{"steps":[]}'),
  listScripts: vi.fn(async () => [] as [string, string][]),
  saveScript: vi.fn(async () => undefined),
}));

afterEach(cleanup);
beforeEach(() => vi.clearAllMocks());

describe("ScriptsPanel Save", () => {
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

    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    expect(
      await screen.findByText(/Không lưu được: expected `steps` to be an array at line 3/),
    ).toBeInTheDocument();
  });

  it("confirms a save that worked, and reloads the saved list", async () => {
    const api = await import("../api");
    render(<ScriptsPanel onUseInJobs={() => undefined} />);

    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    expect(await screen.findByText("Đã lưu")).toBeInTheDocument();
    await waitFor(() => expect(api.listScripts).toHaveBeenCalledTimes(2));
  });
});
