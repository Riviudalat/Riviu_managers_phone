import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { FlowJsonDialog } from "./FlowJsonDialog";
import type { CompiledRevision, FlowDocumentV2 } from "../../types";

/**
 * These tests exist because of one shape: `flow_validate` is the only command in the app that
 * rejects with a **`Vec<CommandError>`** (`flow_commands.rs:69`) — an *array* of objects. This
 * dialog used to render the rejection with `String(reason)`, so a flow that failed to compile
 * showed the operator `[object Object]` instead of which node was wrong, and no test noticed
 * because this file had none.
 */

vi.mock("../../api", () => ({
  flowValidate: vi.fn(),
  flowExport: vi.fn(),
}));

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const document: FlowDocumentV2 = {
  schemaVersion: 2,
  id: "11111111-1111-4111-8111-111111111111",
  name: "Fixture flow",
  revision: 3,
  entryNodeId: "start",
  nodes: [],
  edges: [],
  viewport: { x: 0, y: 0, zoom: 1 },
};

function open(overrides: Partial<Parameters<typeof FlowJsonDialog>[0]> = {}) {
  // Returns the render result: the cases below re-render with a newer document to stand in for an
  // external revision arriving while the dialog is open.
  return render(
    <FlowJsonDialog
      document={overrides.document ?? document}
      onApply={overrides.onApply ?? (() => undefined)}
      onClose={overrides.onClose ?? (() => undefined)}
      validate={overrides.validate ?? (async () => ({}) as CompiledRevision)}
      exportFlow={overrides.exportFlow ?? (async () => "{}")}
    />,
  );
}

const applyButton = () => screen.getByRole("button", { name: /Validate and apply/ });

describe("FlowJsonDialog", () => {
  it("names the failing nodes when validation rejects with a list of issues", async () => {
    open({
      validate: async () => {
        // The exact shape the Rust side rejects with.
        throw [
          { code: "UnknownAction", message: "node tap-1 dùng action không có trong catalog" },
          { code: "DanglingEdge", message: "edge e2 trỏ tới node không tồn tại" },
        ];
      },
    });

    fireEvent.click(applyButton());

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("UnknownAction: node tap-1 dùng action không có trong catalog");
    expect(alert).toHaveTextContent("DanglingEdge: edge e2 trỏ tới node không tồn tại");
    // The whole point.
    expect(alert.textContent).not.toContain("[object Object]");
  });

  it("reads a single Tauri rejection object, not just a list", async () => {
    open({
      validate: async () => {
        throw { code: "FlowCompileFailed", message: "entry node không có trong nodes" };
      },
    });

    fireEvent.click(applyButton());

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("FlowCompileFailed: entry node không có trong nodes");
    expect(alert.textContent).not.toContain("[object Object]");
  });

  it("still reports the local failures that never reach the backend", async () => {
    // `assertFlowDocumentShape` and `JSON.parse` throw real `Error`s; the fix must not have
    // traded one broken message for another.
    open();
    fireEvent.change(screen.getByLabelText("JSON tài liệu"), { target: { value: "{ not json" } });
    fireEvent.click(applyButton());

    await waitFor(() => expect(screen.getByRole("alert")).toBeVisible());
    expect(screen.getByRole("alert").textContent).not.toContain("[object Object]");
    expect(screen.getByRole("alert").textContent).toBeTruthy();
  });

  it("applies a document that validates, and asks the backend exactly once", async () => {
    const onApply = vi.fn();
    const validate = vi.fn(async () => ({}) as CompiledRevision);
    open({ onApply, validate });

    fireEvent.click(applyButton());

    await waitFor(() => expect(onApply).toHaveBeenCalledTimes(1));
    expect(validate).toHaveBeenCalledTimes(1);
    expect(onApply.mock.calls[0][0]).toMatchObject({ id: document.id, revision: 3 });
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("reports a failed export the same way", async () => {
    open({
      exportFlow: async () => {
        throw { code: "NotFound", message: "revision 3 không còn trong DB" };
      },
    });

    fireEvent.click(screen.getByRole("button", { name: /Load saved export/ }));

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("NotFound: revision 3 không còn trong DB");
    expect(alert.textContent).not.toContain("[object Object]");
  });
});

describe("a validation that lands after the operator moved on", () => {
  it("does not apply the click-time JSON once the textarea has changed", async () => {
    // Validation is asynchronous while the textarea stays live. With nothing binding the result to
    // the text that was submitted, the older request applied its own snapshot and the newer visible
    // edits disappeared.
    // A deferred resolver on an object, not a `let`: TypeScript narrows a `let`
    // assigned only inside a closure to `never` at the call site.
    const gate: { release?: (value: CompiledRevision) => void } = {};
    const validate = vi.fn(
      () => new Promise<CompiledRevision>((resolve) => {
        gate.release = resolve;
      }),
    );
    const onApply = vi.fn();
    open({ onApply, validate });

    const editor = screen.getByLabelText("JSON tài liệu");
    const first = JSON.stringify({ ...document, name: "first" });
    fireEvent.change(editor, { target: { value: first } });
    fireEvent.click(applyButton());
    await waitFor(() => expect(validate).toHaveBeenCalledTimes(1));

    fireEvent.change(editor, { target: { value: JSON.stringify({ ...document, name: "second" }) } });
    gate.release?.({} as CompiledRevision);

    await waitFor(() => expect(screen.getByRole("alert")).toBeVisible());
    expect(onApply).not.toHaveBeenCalled();
  });

  it("does not apply after the dialog is closed", async () => {
    // A deferred resolver on an object, not a `let`: TypeScript narrows a `let`
    // assigned only inside a closure to `never` at the call site.
    const gate: { release?: (value: CompiledRevision) => void } = {};
    const validate = vi.fn(
      () => new Promise<CompiledRevision>((resolve) => {
        gate.release = resolve;
      }),
    );
    const onApply = vi.fn();
    const onClose = vi.fn();
    open({ onApply, onClose, validate });
    fireEvent.change(screen.getByLabelText("JSON tài liệu"), {
      target: { value: JSON.stringify(document) },
    });
    fireEvent.click(applyButton());
    await waitFor(() => expect(validate).toHaveBeenCalledTimes(1));

    fireEvent.click(screen.getByRole("button", { name: "Đóng hộp thoại JSON" }));
    gate.release?.({} as CompiledRevision);
    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
    expect(onApply).not.toHaveBeenCalled();
  });

  it("keeps the operator's text when the flow changes underneath, and says so", () => {
    // The reset effect used to follow the document unconditionally, so a `flowUpdated`
    // invalidation arriving mid-edit wiped the textarea with no message.
    const view = open({});
    const editor = screen.getByLabelText("JSON tài liệu");
    fireEvent.change(editor, { target: { value: JSON.stringify({ ...document, name: "mine" }) } });

    view.rerender(
      <FlowJsonDialog
        document={{ ...document, revision: document.revision + 1, name: "theirs" }}
        onApply={vi.fn()}
        onClose={vi.fn()}
        validate={vi.fn()}
        exportFlow={vi.fn()}
      />,
    );

    expect((screen.getByLabelText("JSON tài liệu") as HTMLTextAreaElement).value)
      .toContain('"mine"');
    expect(screen.getByRole("alert").textContent).toContain("cập nhật ở nơi khác");
  });

  it("still follows the document when the operator has not typed anything", () => {
    const view = open({});
    view.rerender(
      <FlowJsonDialog
        document={{ ...document, revision: document.revision + 1, name: "theirs" }}
        onApply={vi.fn()}
        onClose={vi.fn()}
        validate={vi.fn()}
        exportFlow={vi.fn()}
      />,
    );
    expect((screen.getByLabelText("JSON tài liệu") as HTMLTextAreaElement).value)
      .toContain('"theirs"');
    expect(screen.queryByRole("alert")).toBeNull();
  });
});
