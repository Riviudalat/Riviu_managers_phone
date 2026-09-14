import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { StrictMode } from "react";
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

const importButton = () => screen.getByRole("button", { name: /Nhập/ });

describe("FlowImportDialog", () => {
  it("refuses an oversized paste before it ever reaches the backend", async () => {
    // The V2 dialog has had this ceiling all along; the legacy door did not, so a huge
    // paste was held by React, copied across IPC and parsed in full before anything could
    // say no. Same limit, same code, refused here first.
    const importLegacy = vi.fn(async () => ({ document, diagnostics: [] }));
    render(
      <FlowImportDialog onImport={vi.fn()} onClose={() => undefined} importLegacy={importLegacy} />,
    );
    fireEvent.change(screen.getByLabelText("JSON script cũ"), {
      target: { value: `{"version":1,"steps":["${"a".repeat(1_048_576)}"]}` },
    });

    fireEvent.click(importButton());

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("FlowImportTooLarge");
    expect(importLegacy).not.toHaveBeenCalled();
  });

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
    const summary = screen.getByText("Bước 3: Không thể nhập hành động này.");
    expect(summary).toBeVisible();
    expect(summary.closest("details")).not.toHaveAttribute("open");
    expect(summary.closest("details")).toHaveTextContent(
      "UnsupportedAction: action 'shell' không có bản v2",
    );
    expect(onImport).not.toHaveBeenCalled();
  });
});

describe("an import the operator cancelled", () => {
  it("does not replace the open document when the backend answers after Hủy", async () => {
    // Both close controls stay enabled while the conversion runs, and the textarea stays editable.
    // With no lifetime guard, clicking Import and then Hủy still replaced the document the moment
    // the backend answered -- an explicitly cancelled import applying itself.
    // A deferred resolver on an object, not a `let`: TypeScript narrows a `let`
    // assigned only inside a closure to `never` at the call site.
    const gate: { release?: (value: LegacyImportResult) => void } = {};
    const importLegacy = vi.fn(
      () => new Promise<LegacyImportResult>((resolve) => {
        gate.release = resolve;
      }),
    );
    const onImport = vi.fn();
    const onClose = vi.fn();
    render(
      <FlowImportDialog onImport={onImport} onClose={onClose} importLegacy={importLegacy} />,
    );
    fireEvent.change(screen.getByLabelText("JSON script cũ"), {
      target: { value: '{"version":1,"steps":[]}' },
    });
    fireEvent.click(screen.getByRole("button", { name: /Nhập/ }));
    await waitFor(() => expect(importLegacy).toHaveBeenCalledTimes(1));

    fireEvent.click(screen.getByRole("button", { name: "Hủy" }));
    expect(onClose).toHaveBeenCalledTimes(1);

    gate.release?.({ document, diagnostics: [] });
    await waitFor(() => expect(importLegacy).toHaveBeenCalledTimes(1));
    expect(onImport).not.toHaveBeenCalled();
  });

  it("numbers diagnostics the way the operator counts steps", async () => {
    // `step_index` comes from `enumerate()`, so it is zero-based. Printed raw, an error in the
    // second step read "Step 1" and sent the operator to the step before the broken one.
    const importLegacy = vi.fn(async (): Promise<LegacyImportResult> => ({
      document: null,
      diagnostics: [
        { stepIndex: 1, code: "LegacyShapeUnsupported", message: "không nhập được", field: "action" },
      ],
    }));
    open(importLegacy);
    fireEvent.click(screen.getByRole("button", { name: /Nhập/ }));
    const summary = await screen.findByText("Bước 2: Không thể nhập hành động này.");
    expect(summary.closest("details")).toHaveTextContent(
      "LegacyShapeUnsupported: không nhập được",
    );
  });
});

describe("the lifetime guard survives a StrictMode remount", () => {
  it("still applies a successful import when React double-mounts the effect", async () => {
    // The guard was written as `useEffect(() => () => { live.current = false }, [])` -- cleanup
    // only. StrictMode mounts, unmounts and remounts every effect, so the cleanup ran and nothing
    // set the flag back: the guard then rejected every result for the rest of the component's life
    // and Import silently stopped working. jsdom tests do not wrap in StrictMode, so only the e2e
    // suite caught it. This case closes that gap for all three dialogs that use the pattern.
    const importLegacy = vi.fn(async (): Promise<LegacyImportResult> => ({
      document,
      diagnostics: [],
    }));
    const onImport = vi.fn();
    render(
      <StrictMode>
        <FlowImportDialog
          onImport={onImport}
          onClose={() => undefined}
          importLegacy={importLegacy}
        />
      </StrictMode>,
    );
    fireEvent.change(screen.getByLabelText("JSON script cũ"), {
      target: { value: '{"version":1,"steps":[]}' },
    });
    fireEvent.click(screen.getByRole("button", { name: /Nhập/ }));

    await waitFor(() => expect(onImport).toHaveBeenCalledWith(document));
  });
});

describe("workflow conversion preview", () => {
  const exported = (action = "Pause", options = { timeoutType: "fixed", timeout: 2 }) => JSON.stringify({ name: "Sample flow", script: { nodes: [
    { id: "start", data: { action: "Start", options: {}, successNode: "step" } },
    { id: "step", data: { action, options, successNode: "end" } },
    { id: "end", data: { action: "Stop", options: {} } },
  ] } });

  function openSource(format: "genfarmer" | "macro", value: string) {
    const onImport = vi.fn();
    const importLegacy = vi.fn();
    render(<FlowImportDialog onImport={onImport} onClose={vi.fn()} importLegacy={importLegacy} />);
    fireEvent.change(screen.getByLabelText("Định dạng nguồn"), { target: { value: format } });
    fireEvent.change(screen.getByLabelText("JSON nguồn"), { target: { value } });
    return { onImport, importLegacy };
  }

  it("shows converted counts and source mappings before accepting a draft", () => {
    const { onImport, importLegacy } = openSource("genfarmer", exported());
    fireEvent.click(screen.getByRole("button", { name: "Xem trước" }));
    expect(screen.getByLabelText("Xem trước chuyển đổi")).toHaveTextContent("3 bước nguồn");
    expect(screen.getByLabelText("Xem trước chuyển đổi")).toHaveTextContent("3 node trong bản nháp");
    expect(onImport).not.toHaveBeenCalled();
    expect(importLegacy).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Mở bản nháp" }));
    expect(onImport).toHaveBeenCalledWith(expect.objectContaining({ revision: 0, name: "Sample flow" }));
  });

  it("shows the source ID and prevents applying a partially supported script", () => {
    const { onImport } = openSource("genfarmer", exported("Javascript"));
    fireEvent.click(screen.getByRole("button", { name: "Xem trước" }));
    expect(screen.getByLabelText("Xem trước chuyển đổi")).toHaveTextContent("Bước 2 [step]");
    expect(screen.getByLabelText("Xem trước chuyển đổi")).toHaveTextContent("UnsupportedAction");
    expect(screen.queryByRole("button", { name: "Mở bản nháp" })).toBeNull();
    expect(onImport).not.toHaveBeenCalled();
  });

  it("invalidates an accepted preview immediately when source text changes", () => {
    openSource("genfarmer", exported());
    fireEvent.click(screen.getByRole("button", { name: "Xem trước" }));
    expect(screen.getByRole("button", { name: "Mở bản nháp" })).toBeEnabled();
    fireEvent.change(screen.getByLabelText("JSON nguồn"), { target: { value: exported("Javascript") } });
    expect(screen.queryByRole("button", { name: "Mở bản nháp" })).toBeNull();
  });

  it("previews macro waits and explains missing geometry for old taps", () => {
    const { onImport } = openSource("macro", JSON.stringify({ id: "m", name: "Recorded", steps: [{ kind: "tap", x: 20, y: 30, iw: 400, ih: 800, afterMs: 100 }] }));
    fireEvent.click(screen.getByRole("button", { name: "Xem trước" }));
    expect(screen.getByLabelText("Xem trước chuyển đổi")).toHaveTextContent("CalibrationRequired");
    expect(onImport).not.toHaveBeenCalled();
    fireEvent.change(screen.getByLabelText("JSON nguồn"), { target: { value: JSON.stringify({ id: "m", name: "Recorded", steps: [{ kind: "wait", afterMs: 100 }] }) } });
    fireEvent.click(screen.getByRole("button", { name: "Xem trước" }));
    expect(screen.getByRole("button", { name: "Mở bản nháp" })).toBeEnabled();
  });

  it("reads a chosen JSON file into preview without importing or invoking the runtime", async () => {
    const { onImport, importLegacy } = openSource("genfarmer", "");
    const file = new File([exported()], "sample.json", { type: "application/json" });
    Object.defineProperty(file, "text", { value: async () => exported() });
    fireEvent.change(screen.getByLabelText("Tệp JSON (tối đa 1 MiB)"), { target: { files: [file] } });
    await waitFor(() => expect(screen.getByLabelText("JSON nguồn")).toHaveValue(exported()));
    fireEvent.click(screen.getByRole("button", { name: "Xem trước" }));
    expect(screen.getByRole("button", { name: "Mở bản nháp" })).toBeEnabled();
    expect(onImport).not.toHaveBeenCalled();
    expect(importLegacy).not.toHaveBeenCalled();
  });

  it("rejects an oversized file before reading it", async () => {
    openSource("genfarmer", "");
    const text = vi.fn();
    const file = new File([], "large.json");
    Object.defineProperties(file, { text: { value: text }, size: { value: 1_048_577 } });
    fireEvent.change(screen.getByLabelText("Tệp JSON (tối đa 1 MiB)"), { target: { files: [file] } });
    expect(await screen.findByRole("alert")).toHaveTextContent("FlowImportTooLarge");
    expect(text).not.toHaveBeenCalled();
  });

  it("does not apply a legacy response after editing or switching source format", async () => {
    let release: (value: LegacyImportResult) => void = () => undefined;
    const imported = new Promise<LegacyImportResult>((resolve) => { release = resolve; });
    const onImport = open(() => imported);
    fireEvent.click(importButton());
    fireEvent.change(screen.getByLabelText("Định dạng nguồn"), { target: { value: "genfarmer" } });
    release({ document, diagnostics: [] });
    await waitFor(() => expect(screen.getByRole("button", { name: "Xem trước" })).toBeDisabled());
    expect(onImport).not.toHaveBeenCalled();
  });
});
