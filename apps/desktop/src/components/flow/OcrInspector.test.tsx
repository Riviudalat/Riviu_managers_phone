import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { ActionDefinition, FlowNode } from "../../types";
import { FlowInspector } from "./FlowInspector";

afterEach(cleanup);
const node: FlowNode = { id: "ocr-node", kind: "ocrReadText", position: { x: 0, y: 0 },
  config: { name: "caption", languages: ["vi", "en"], minConfidence: 0.7 }, postcondition: null };
const definition: ActionDefinition = {
  kind: "ocrReadText", schemaVersion: 1, label: "Read OCR", disabledReason: null, category: "evidence",
  configSchema: { type: "object", properties: {} }, inputPorts: [], outputPorts: [],
  requiredCapabilities: ["stream"], resourceClass: "uiWithStream", sideEffectClass: "none",
  evidenceRequirement: "none", allowedEvidence: [], qualifiedDetectorIds: [], reconciliationPolicy: "none",
  defaultTimeoutMs: 30000, retryPolicy: "beforeDispatchOnly",
};

it("writes OCR language, threshold, variable and normalized ROI settings into the actual node config", () => {
  const change = vi.fn();
  render(<FlowInspector node={node} definition={definition} issues={[]} onConfigChange={change}
    onPostconditionChange={vi.fn()} coordinateDeviceUdid={null} launchBundleId={null} />);
  fireEvent.change(screen.getByLabelText("Biến lưu nội dung"), { target: { value: "title" } });
  expect(change).toHaveBeenLastCalledWith({ ...node.config, name: "title" });
  fireEvent.change(screen.getByLabelText("Độ tin cậy tối thiểu"), { target: { value: "0.95" } });
  expect(change).toHaveBeenLastCalledWith({ ...node.config, minConfidence: 0.95 });
  fireEvent.click(screen.getByLabelText("Tiếng Anh"));
  expect(change).toHaveBeenLastCalledWith({ ...node.config, languages: ["vi"] });
  fireEvent.click(screen.getByLabelText("Giới hạn vùng đọc chữ"));
  expect(change).toHaveBeenLastCalledWith({ ...node.config, region: { x0: 0, y0: 0, x1: 1, y1: 1 } });
  expect(screen.getByRole("button", { name: "Chụp ảnh kiểm tra OCR" })).toBeDisabled();
});
