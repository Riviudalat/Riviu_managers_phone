import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { defaultConfigForAction, createFlowNode } from "../../flow/model";
import type { ActionDefinition, ActionKind, FlowNodeAttemptRecord, FlowRunDetail, JsonObject, JsonValue } from "../../types";
import { FlowActionNode } from "./FlowActionNode";
import { FlowInspector } from "./FlowInspector";
import { FlowRunMonitor } from "./FlowRunMonitor";

vi.mock("@xyflow/react", () => ({ Position: { Left: "left", Right: "right" }, Handle: ({ id, type }: { id: string; type: string }) => <span data-testid={`${type}-${id}`} /> }));
vi.mock("../../api", () => ({ flowGetRun: vi.fn(), listenRiviuEvents: vi.fn(async () => vi.fn()) }));
afterEach(cleanup);

const locator: JsonValue = { type: "object", required: ["strategy", "value"], properties: { strategy: { type: "string", enum: ["accessibilityId", "className"] }, value: { type: "string" } } };
const properties: Record<string, JsonObject> = {
  ifVisible: { locator },
  readText: { name: { type: "string" }, locator },
  setVariable: { name: { type: "string" }, value: { type: "string" } },
  ifValue: { name: { type: "string" }, operator: { type: "string", enum: ["equals", "notEquals", "contains", "startsWith", "isEmpty", "notEmpty"] }, value: { type: "string" } },
  log: { message: { type: "string" }, variable: { type: "string" } },
};
function definition(kind: ActionKind): ActionDefinition {
  return { kind, schemaVersion: 1, label: kind, disabledReason: null, category: "control", configSchema: { type: "object", properties: properties[kind] }, inputPorts: [], outputPorts: [], requiredCapabilities: [], resourceClass: "pureDesktop", sideEffectClass: "none", evidenceRequirement: "none", allowedEvidence: [], qualifiedDetectorIds: [], reconciliationPolicy: "none", defaultTimeoutMs: 5000, retryPolicy: "beforeDispatchOnly" };
}
function Inspector({ kind }: { kind: ActionKind }) {
  const [node, setNode] = useState(() => createFlowNode(kind, { x: 0, y: 0 }, () => "fixture"));
  return <><FlowInspector node={node} definition={definition(kind)} issues={[]} onConfigChange={(config) => setNode({ ...node, config })} onPostconditionChange={vi.fn()} /><output data-testid="edited-config">{JSON.stringify(node.config)}</output></>;
}

describe("Flow data action authoring", () => {
  it.each([
    ["ifVisible", { locator: { strategy: "accessibilityId", value: "" } }],
    ["readText", { name: "observedText", locator: { strategy: "accessibilityId", value: "" } }],
    ["setVariable", { name: "message", value: "" }],
    ["ifValue", { name: "observedText", operator: "equals", value: "" }],
    ["log", { message: "Đã đến bước này", variable: "" }],
  ] as const)("initializes %s with an editable structured config", (kind, expected) => {
    expect(defaultConfigForAction(kind)).toEqual(expected);
    expect(createFlowNode(kind, { x: 0, y: 0 }).postcondition).toBeNull();
  });

  it.each(["ifVisible", "readText"] as const)("edits %s locator strategy and value without losing its other fields", (kind) => {
    render(<Inspector kind={kind} />);
    fireEvent.change(screen.getByLabelText("Cách định vị"), { target: { value: "className" } });
    fireEvent.change(screen.getByLabelText("Giá trị nhận diện"), { target: { value: "android.widget.EditText" } });
    const config = JSON.parse(screen.getByTestId("edited-config").textContent!);
    expect(config.locator).toEqual({ strategy: "className", value: "android.widget.EditText" });
    if (kind === "readText") {
      fireEvent.change(screen.getByLabelText("Tên biến"), { target: { value: "caption" } });
      expect(JSON.parse(screen.getByTestId("edited-config").textContent!)).toEqual({ ...config, name: "caption" });
    }
  });

  it("keeps a literal assigned value as text, including variable-like expressions", () => {
    render(<Inspector kind="setVariable" />);
    fireEvent.change(screen.getByLabelText("Tên biến"), { target: { value: "message" } });
    fireEvent.change(screen.getByLabelText("Giá trị nhận diện"), { target: { value: "${other} <script>plain</script>" } });
    expect(JSON.parse(screen.getByTestId("edited-config").textContent!)).toEqual({ name: "message", value: "${other} <script>plain</script>" });
  });

  it("offers all comparisons in Vietnamese and preserves the chosen runtime operator", () => {
    render(<Inspector kind="ifValue" />);
    const operators = screen.getByLabelText("Phép so sánh");
    expect(operators).toHaveTextContent("BằngKhácChứaBắt đầu bằngRỗngCó nội dung");
    fireEvent.change(operators, { target: { value: "contains" } });
    fireEvent.change(screen.getByLabelText("Giá trị nhận diện"), { target: { value: "Đã xong" } });
    expect(JSON.parse(screen.getByTestId("edited-config").textContent!)).toEqual({ name: "observedText", operator: "contains", value: "Đã xong" });
  });

  it("edits log message and optional variable independently", () => {
    render(<Inspector kind="log" />);
    fireEvent.change(screen.getByLabelText("Thông điệp nhật ký"), { target: { value: "Đã đọc dữ liệu" } });
    fireEvent.change(screen.getByLabelText("Biến đính kèm (tùy chọn)"), { target: { value: "caption" } });
    expect(JSON.parse(screen.getByTestId("edited-config").textContent!)).toEqual({ message: "Đã đọc dữ liệu", variable: "caption" });
  });

  it.each(["ifVisible", "ifValue"] as const)("renders both real %s branch ports", (kind) => {
    render(<FlowActionNode id="node" type="flowAction" data={{ kind, config: defaultConfigForAction(kind), issues: [] }} selected={false} isConnectable dragging={false} zIndex={0} positionAbsoluteX={0} positionAbsoluteY={0} draggable selectable deletable />);
    expect(screen.getByTestId("target-flow")).toBeInTheDocument();
    expect(screen.getByTestId("source-matched")).toBeInTheDocument();
    expect(screen.getByTestId("source-notMatched")).toBeInTheDocument();
    expect(screen.queryByTestId("source-flow")).toBeNull();
    expect(screen.getByText("khớp")).toBeVisible();
    expect(screen.getByText("không khớp")).toBeVisible();
  });
});

function attempt(kind: ActionKind, evidenceResult: JsonValue, chosenPort?: string): FlowNodeAttemptRecord {
  return { id: `attempt-${kind}`, deviceRunId: "dr", nodeId: `node-${kind}`, actionKind: kind, attemptNo: 1, sideEffectClass: "none", state: "succeeded", canonicalInput: null, evidenceBaseline: null, evidenceResult, chosenPort, retryAllowed: false, error: null, startedAt: "2026-09-13T00:00:00Z", updatedAt: "2026-09-13T00:00:01Z", finishedAt: "2026-09-13T00:00:01Z" };
}
function run(attempts: FlowNodeAttemptRecord[]): FlowRunDetail {
  return { run: { id: "r", flowId: "f", flowRevision: 1, planSha256: "a".repeat(64), selection: { requested: { mode: "selected", udids: ["device"] }, targetUdids: ["device"] }, state: "succeeded", eventRevision: 1, error: null, createdAt: "2026-09-13T00:00:00Z", updatedAt: "2026-09-13T00:00:01Z" }, deviceRuns: [{ id: "dr", runId: "r", udid: "device", state: "succeeded", capabilitySnapshot: null, releaseProof: null, error: null, startedAt: null, finishedAt: null }], attempts, artifacts: [] };
}

describe("Flow data execution evidence", () => {
  it("shows the actual nested revision and iteration from persisted source metadata",()=>{
    const detail=run([attempt("log",{kind:"flowLog",message:"iteration output"})]);
    detail.sourcePaths={"node-log":{sourceNodeId:"source-original",path:[{nodeId:"repeat-parent",flowId:"child",revision:4,iteration:3},{nodeId:"subflow",flowId:"nested",revision:2,iteration:null}]}};
    render(<FlowRunMonitor run={detail} onCancel={vi.fn()} onRetry={vi.fn()}/>);
    expect(screen.getByText("Flow con · bản 4 / lượt 3 → Flow con · bản 2")).toBeVisible();
    expect(screen.getByText("source-original")).not.toBeVisible();
    const disclosure=screen.getByRole("group",{name:"Chi tiết kỹ thuật bước"});fireEvent.click(disclosure.querySelector("summary")!);
    expect(screen.getByText("source-original")).toBeVisible();
  });
  it("keeps scoped variable IDs inside technical evidence and displays their value clearly",()=>{
    const detail=run([attempt("readText",{kind:"flowVariable",name:`v_${"a".repeat(32)}`,value:"ready"})]);
    detail.sourcePaths={"node-readText":{sourceNodeId:"original",path:[{nodeId:"parent",flowId:"child",revision:2,iteration:1}]}};
    render(<FlowRunMonitor run={detail} onCancel={vi.fn()} onRetry={vi.fn()}/>);
    expect(screen.getByText("Biến Flow con = ready")).toBeVisible();
    expect(screen.queryByText(`v_${"a".repeat(32)} = ready`)).toBeNull();
  });
  it("shows persisted read/assign/log values as visible text rather than hidden JSON only", () => {
    render(<FlowRunMonitor run={run([
      attempt("readText", { kind: "flowVariable", name: "caption", value: "Đã đăng" }),
      attempt("setVariable", { kind: "flowVariable", name: "literal", value: "<img src=x>" }),
      attempt("log", { kind: "flowLog", message: "Đã đọc", value: "Đã đăng" }),
    ])} onCancel={vi.fn()} onRetry={vi.fn()} />);
    expect(screen.getByText("caption = Đã đăng")).toBeVisible();
    expect(screen.getByText("literal = <img src=x>")).toBeVisible();
    expect(screen.getByText("Đã đọc · Đã đăng")).toBeVisible();
    expect(screen.queryByRole("img")).toBeNull();
    expect(screen.queryByRole("button", { name: /Chạy lại/ })).toBeNull();
  });

  it("distinguishes the persisted matched and notMatched branches", () => {
    render(<FlowRunMonitor run={run([
      attempt("ifVisible", { kind: "flowPredicate", matched: true }, "matched"),
      attempt("ifValue", { kind: "flowPredicate", matched: false }, "notMatched"),
    ])} onCancel={vi.fn()} onRetry={vi.fn()} />);
    expect(screen.getByText(/nhánh Khớp/)).toBeVisible();
    expect(screen.getByText(/nhánh Không khớp/)).toBeVisible();
  });
});
