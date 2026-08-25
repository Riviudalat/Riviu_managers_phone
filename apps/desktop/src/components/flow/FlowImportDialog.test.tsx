import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { FlowImportDialog } from "./FlowImportDialog";
import type { FlowDocumentV2, LegacyImportResult } from "../../types";

/**
 * `flow_import_legacy` rejects with a `CommandError` object (`flow_commands.rs:111`), and this
 * dialog rendered it with `String(reason)` — so an operator pasting a script the backend refused
 * read `[object Object]` where the reason should be. The file had no test, which is why it
 * survived a sweep that fixed 47 other sites.
 */

vi.mock("../../api", () => ({ flowImportLegacy: vi.fn() }));

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const document: FlowDocumentV2 = {
  schemaVersion: 2,
  id: "22222222-2222-4222-8222-222222222222",
  name: "Imported",
  revision: 0,
  entryNodeId: "start",
  nodes: [],
  edges: [],
  viewport: { x: 0, y: 0, zoom: 1 },
};

function open(importLegacy: (scriptJson: string) => Promise<LegacyImportResult>, onImport = vi.fn()) {
  render(
    <FlowImportDialog onImport={onImport} onClose={() => undefined} importLegacy={importLegacy} />,
  );
  fireEvent.change(screen.getByLabelText("JSON script cũ"), {
    target: { value: '{"version":1,"steps":[]}' },
  });
  return onImport;
}

const importButton = () => screen.getByRole("button", { name: /Import/ });

describe("FlowImportDialog", () => {
  it("reads the reason out of a Tauri rejection instead of stringifying the object", async () => {
    open(async () => {
      throw { code: "InvalidArgument", message: "legacy script JSON is invalid" };
    });

    fireEvent.click(importButton());

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("InvalidArgument: legacy script JSON is invalid");
    expect(alert.textContent).not.toContain("[object Object]");
  });

  it("imports a clean script and keeps the dialog quiet", async () => {
    const onImport = open(async () => ({ document, diagnostics: [] }));

    fireEvent.click(importButton());

    await waitFor(() => expect(onImport).toHaveBeenCalledWith(document));
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("shows diagnostics and does NOT import when the script only partly converts", async () => {
    // The half-success case: a document came back, but with complaints. Importing it anyway
    // would hand the operator a flow that quietly dropped steps.
    const onImport = open(async () => ({
      document,
      diagnostics: [
        { stepIndex: 2, code: "UnsupportedAction", message: "action 'shell' không có bản v2", field: null },
      ],
    }));

    fireEvent.click(importButton());

    await waitFor(() => expect(screen.getByLabelText("Chẩn đoán nhập")).toBeVisible());
    expect(screen.getByText(/UnsupportedAction/)).toBeVisible();
    expect(screen.getByText(/action 'shell' không có bản v2/)).toBeVisible();
    expect(onImport).not.toHaveBeenCalled();
  });
});
