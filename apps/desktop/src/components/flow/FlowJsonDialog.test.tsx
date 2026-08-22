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
  render(
    <FlowJsonDialog
      document={document}
      onApply={overrides.onApply ?? (() => undefined)}
      onClose={() => undefined}
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
