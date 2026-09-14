import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { RepeatFlowDialog } from "./RepeatFlowDialog";
import type { FlowDocumentV2 } from "../../types";

afterEach(cleanup);
const document: FlowDocumentV2 = { schemaVersion: 2, id: "f", revision: 1, name: "Routine", entryNodeId: "s", viewport: { x: 0, y: 0, zoom: 1 }, nodes: [
  { id: "s", kind: "start", config: {}, position: { x: 0, y: 0 } },
  { id: "w", kind: "wait", config: { durationMs: 500 }, position: { x: 240, y: 0 } },
  { id: "e", kind: "end", config: {}, position: { x: 480, y: 0 } },
], edges: [["s", "w"], ["w", "e"]].map(([source, target], index) => ({ id: `e${index}`, sourceNodeId: source, sourcePort: "flow", targetNodeId: target, targetPort: "flow" })) };

describe("repeat dialog", () => {
  it("previews total repetitions and explicit wait time before editing", () => {
    const onApply = vi.fn();
    render(<RepeatFlowDialog document={document} onApply={onApply} onClose={vi.fn()} />);
    expect(screen.getByLabelText("Kết quả lặp")).toHaveTextContent("1 hành động × 2 lượt = 4 node");
    expect(screen.getByLabelText("Kết quả lặp")).toHaveTextContent("1 giây");
    expect(onApply).not.toHaveBeenCalled();
    fireEvent.change(screen.getByLabelText("Tổng số lượt"), { target: { value: "3" } });
    fireEvent.click(screen.getByRole("button", { name: "Tạo các lượt" }));
    expect(onApply.mock.calls[0][0].nodes).toHaveLength(5);
  });

  it("disables invalid counts, no-op count and unsupported branches", () => {
    const { rerender } = render(<RepeatFlowDialog document={document} onApply={vi.fn()} onClose={vi.fn()} />);
    fireEvent.change(screen.getByLabelText("Tổng số lượt"), { target: { value: "51" } });
    expect(screen.getByRole("alert")).toHaveTextContent("1 đến 50");
    expect(screen.getByRole("button", { name: "Tạo các lượt" })).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Tổng số lượt"), { target: { value: "1" } });
    expect(screen.getByRole("button", { name: "Tạo các lượt" })).toBeDisabled();
    const branch = structuredClone(document);
    branch.nodes[1].kind = "ifVision";
    rerender(<RepeatFlowDialog document={branch} onApply={vi.fn()} onClose={vi.fn()} />);
    expect(screen.getByRole("alert")).toHaveTextContent("nhánh điều kiện");
  });

  it("cancels without applying", () => {
    const onApply = vi.fn();
    const onClose = vi.fn();
    render(<RepeatFlowDialog document={document} onApply={onApply} onClose={onClose} />);
    fireEvent.click(screen.getByRole("button", { name: "Hủy" }));
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(onApply).not.toHaveBeenCalled();
  });
});
