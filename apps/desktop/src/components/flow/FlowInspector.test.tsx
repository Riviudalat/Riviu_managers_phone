import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type {
  ActionDefinition,
  ActionKind,
  CompiledRevision,
  EvidenceKind,
  EvidenceSpec,
  FlowCoordinateFrame,
  FlowDocumentV2,
  FlowNode,
  FlowValidationIssue,
  JsonObject,
  JsonValue,
  LegacyImportResult,
} from "../../types";
import { newFlowDocument } from "./editorState";
import { FlowCoordinatePicker } from "./FlowCoordinatePicker";
import { FlowImportDialog } from "./FlowImportDialog";
import { FlowInspector } from "./FlowInspector";
import { FlowJsonDialog } from "./FlowJsonDialog";

afterEach(cleanup);

const coordinateSchema: JsonValue = {
  type: "object",
  properties: {
    x: { type: "number" },
    y: { type: "number" },
    imageWidth: { type: "integer", minimum: 1 },
    imageHeight: { type: "integer", minimum: 1 },
    orientation: {
      type: "string",
      enum: ["portrait", "portraitUpsideDown", "landscapeLeft", "landscapeRight"],
    },
    profileId: { type: "string" },
  },
};

function definition(
  kind: ActionKind,
  configSchema: JsonValue,
  allowedEvidence: EvidenceKind[] = [],
  qualifiedDetectorIds: string[] = [],
): ActionDefinition {
  return {
    kind,
    schemaVersion: 1,
    label: kind,
    disabledReason: null,
    category: kind === "wait" ? "timing" : "input",
    configSchema,
    inputPorts: [],
    outputPorts: [],
    requiredCapabilities: [],
    resourceClass: "pureDesktop",
    sideEffectClass: "none",
    evidenceRequirement: allowedEvidence.length === 0 ? "none" : "frame",
    allowedEvidence,
    qualifiedDetectorIds,
    reconciliationPolicy: "none",
    defaultTimeoutMs: 5_000,
    retryPolicy: "beforeDispatchOnly",
  };
}

function node(
  kind: ActionKind,
  config: JsonObject,
  postcondition: EvidenceSpec | null = null,
): FlowNode {
  return {
    id: `node-${kind}`,
    kind,
    position: { x: 0, y: 0 },
    config,
    postcondition,
  };
}

function InspectorHarness({
  initialNode,
  action,
  issues = [],
  loadCoordinateFrame,
  onConfig = vi.fn(),
  onEvidence = vi.fn(),
}: {
  initialNode: FlowNode;
  action: ActionDefinition;
  issues?: FlowValidationIssue[];
  loadCoordinateFrame?: () => Promise<FlowCoordinateFrame>;
  onConfig?: (config: JsonObject) => void;
  onEvidence?: (evidence: EvidenceSpec | null) => void;
}) {
  const [current, setCurrent] = useState(initialNode);
  return (
    <>
      <FlowInspector
        node={current}
        definition={action}
        issues={issues}
        loadCoordinateFrame={loadCoordinateFrame}
        onConfigChange={(config, postcondition) => {
          onConfig(config);
          // The workspace applies both halves in one dispatch, so the harness has to as well --
          // otherwise these tests watch a state the app never produces.
          if (postcondition !== undefined) onEvidence(postcondition);
          setCurrent((value) => ({
            ...value,
            config,
            ...(postcondition === undefined ? {} : { postcondition }),
          }));
        }}
        onPostconditionChange={(postcondition) => {
          onEvidence(postcondition);
          setCurrent((value) => ({ ...value, postcondition }));
        }}
      />
      <output data-testid="config">{JSON.stringify(current.config)}</output>
      <output data-testid="evidence">{JSON.stringify(current.postcondition)}</output>
    </>
  );
}

function compiled(document: FlowDocumentV2): CompiledRevision {
  return {
    plan: {
      schemaVersion: 2,
      flowId: document.id,
      revision: document.revision,
      nodes: {},
      executionOrder: [],
      contextPlan: {
        requiresExclusive: false,
        requiresUiSession: false,
        requiresStream: false,
        requiresFreshTextSession: false,
        initialBundleId: null,
      },
      actionDefinitionVersions: {},
      requiredCapabilities: [],
    },
    canonicalJson: "{}",
    sha256: "11".repeat(32),
  };
}

describe("FlowInspector", () => {
  it("edits Launch App bundle IDs and exposes only backend-allowed evidence", () => {
    const action = definition(
      "launchApp",
      {
        type: "object",
        properties: { bundleId: { type: "string", minLength: 1, maxLength: 255 } },
      },
      ["activeAppEquals"],
    );
    render(
      <InspectorHarness
        initialNode={node(
          "launchApp",
          { bundleId: "com.fixture.old" },
          { kind: "activeAppEquals", bundleId: "com.fixture.old" },
        )}
        action={action}
      />,
    );

    fireEvent.change(screen.getByLabelText("Bundle ID"), {
      target: { value: "com.fixture.new" },
    });
    expect(screen.getByTestId("config")).toHaveTextContent('"bundleId":"com.fixture.new"');
    expect(screen.getByTestId("evidence")).toHaveTextContent(
      '"bundleId":"com.fixture.new"',
    );
    expect(within(screen.getByLabelText("Loại bằng chứng")).getByRole("option", {
      name: "App đang mở khớp bundle",
    })).toBeInTheDocument();
    expect(screen.queryByRole("option", { name: "Tiến trình đã tắt" })).not.toBeInTheDocument();
  });

  it("keeps Terminate App ProcessAbsent evidence equal to the configured bundle", () => {
    const onEvidence = vi.fn();
    render(
      <InspectorHarness
        initialNode={node(
          "terminateApp",
          { bundleId: "com.fixture.old" },
          { kind: "processAbsent", bundleId: "com.fixture.old" },
        )}
        action={definition(
          "terminateApp",
          { type: "object", properties: { bundleId: { type: "string" } } },
          ["processAbsent"],
        )}
        onEvidence={onEvidence}
      />,
    );
    fireEvent.change(screen.getByLabelText("Bundle ID"), {
      target: { value: "com.fixture.target" },
    });
    expect(onEvidence).toHaveBeenLastCalledWith({
      kind: "processAbsent",
      bundleId: "com.fixture.target",
    });
    expect(screen.getByLabelText("Bundle ID phải vắng mặt")).toHaveValue(
      "com.fixture.target",
    );
  });

  it("accepts only finite bounded integer Wait values", () => {
    const onConfig = vi.fn();
    render(
      <InspectorHarness
        initialNode={node("wait", { durationMs: 1_000 })}
        action={definition("wait", {
          type: "object",
          properties: {
            durationMs: { type: "integer", minimum: 1, maximum: 60_000 },
          },
        })}
        onConfig={onConfig}
      />,
    );
    const input = screen.getByLabelText("Thời lượng (ms)");
    expect(input).toHaveAttribute("min", "1");
    expect(input).toHaveAttribute("max", "60000");
    expect(input).toHaveAttribute("step", "1");

    fireEvent.change(input, { target: { value: "60000" } });
    expect(onConfig).toHaveBeenCalledTimes(1);
    fireEvent.change(input, { target: { value: "60001" } });
    fireEvent.change(input, { target: { value: "1.5" } });
    fireEvent.change(input, { target: { value: "NaN" } });
    expect(onConfig).toHaveBeenCalledTimes(1);
    expect(screen.getByTestId("config")).toHaveTextContent('"durationMs":60000');
  });

  it("keeps Tap target modes exclusive and stores a device-frame pick", async () => {
    const frame: FlowCoordinateFrame = {
      jpegBase64: "fixture-jpeg",
      imageWidth: 375,
      imageHeight: 667,
      orientation: "portrait",
      profileId: "11".repeat(32),
    };
    render(
      <InspectorHarness
        initialNode={node("tap", { accessibilityId: "fixture-button" })}
        action={definition(
          "tap",
          {
            type: "object",
            properties: {
              point: coordinateSchema,
              accessibilityId: { type: "string" },
            },
          },
          ["frameRegionChanged", "qualifiedFramePredicate"],
        )}
        loadCoordinateFrame={vi.fn().mockResolvedValue(frame)}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Toạ độ" }));
    expect(screen.getByTestId("config")).not.toHaveTextContent("accessibilityId");
    fireEvent.click(screen.getByRole("button", { name: "Chọn điểm trên thiết bị" }));
    const image = await screen.findByRole("img", { name: "Device frame" });
    vi.spyOn(image, "getBoundingClientRect").mockReturnValue({
      x: 100,
      y: 50,
      left: 100,
      top: 50,
      right: 475,
      bottom: 717,
      width: 375,
      height: 667,
      toJSON: () => ({}),
    } as DOMRect);
    fireEvent.click(image, { clientX: 287.5, clientY: 383.5 });

    const config = JSON.parse(screen.getByTestId("config").textContent ?? "null");
    expect(config).toEqual({
      point: {
        x: 187.5,
        y: 333.5,
        imageWidth: 375,
        imageHeight: 667,
        orientation: "portrait",
        profileId: "11".repeat(32),
      },
    });
    expect(screen.queryByRole("option", {
      name: "Vị ngữ khung có kiểm định",
    })).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Accessibility ID" }));
    expect(JSON.parse(screen.getByTestId("config").textContent ?? "null")).toEqual({
      accessibilityId: "",
    });
  });

  it("renders finite Swipe coordinates and rejects non-finite edits", () => {
    const onConfig = vi.fn();
    render(
      <InspectorHarness
        initialNode={node("swipe", {
          from: {
            x: 10,
            y: 20,
            imageWidth: 375,
            imageHeight: 667,
            orientation: "portrait",
            profileId: "11".repeat(32),
          },
          to: {
            x: 10,
            y: 500,
            imageWidth: 375,
            imageHeight: 667,
            orientation: "portrait",
            profileId: "11".repeat(32),
          },
          durationMs: 280,
        })}
        action={definition("swipe", {
          type: "object",
          properties: {
            from: coordinateSchema,
            to: coordinateSchema,
            durationMs: { type: "integer", minimum: 1, maximum: 5_000 },
          },
        })}
        onConfig={onConfig}
      />,
    );
    const from = screen.getByRole("group", { name: "Từ điểm" });
    const x = within(from).getByLabelText("X");
    fireEvent.change(x, { target: { value: "42.5" } });
    expect(onConfig).toHaveBeenCalledTimes(1);
    fireEvent.change(x, { target: { value: "Infinity" } });
    expect(onConfig).toHaveBeenCalledTimes(1);
    expect(within(from).getByLabelText("Profile ID")).toHaveAttribute("readonly");
    expect(within(from).getByLabelText("Hướng màn hình")).toHaveValue("portrait");
  });

  it("uses a two-option Type Text read-back locator and synchronizes its evidence", () => {
    render(
      <InspectorHarness
        initialNode={node(
          "typeText",
          {
            text: "hello",
            readBackLocator: { strategy: "accessibilityId", value: "search" },
          },
          {
            kind: "textReadBackEquals",
            locator: { strategy: "accessibilityId", value: "search" },
            value: "hello",
          },
        )}
        action={definition(
          "typeText",
          {
            type: "object",
            properties: {
              text: { type: "string", maxLength: 4_096 },
              readBackLocator: {
                type: "object",
                properties: {
                  strategy: { type: "string", enum: ["accessibilityId", "className"] },
                  value: { type: "string" },
                },
              },
            },
          },
          ["textReadBackEquals"],
        )}
      />,
    );
    const configLocator = screen.getAllByRole("group", { name: "Cách định vị" })[0];
    expect(within(configLocator).getAllByRole("button")).toHaveLength(2);
    fireEvent.click(within(configLocator).getByRole("button", { name: "Class name" }));
    expect(screen.getByTestId("config")).toHaveTextContent('"strategy":"className"');
    expect(screen.getByTestId("evidence")).toHaveTextContent('"strategy":"className"');
    fireEvent.change(screen.getByLabelText("Nội dung"), { target: { value: "updated" } });
    expect(screen.getByTestId("evidence")).toHaveTextContent('"value":"updated"');
  });

  it("bounds screenshot labels and maps field and node issues", () => {
    const onConfig = vi.fn();
    const action = definition("screenshot", {
      type: "object",
      properties: {
        label: { type: "string", minLength: 1, maxLength: 96 },
        format: { type: "string", enum: ["jpeg"] },
      },
    }, ["artifactDecodedAndHashed"]);
    const screenshotNode = node("screenshot", { label: "capture", format: "jpeg" });
    render(
      <InspectorHarness
        initialNode={screenshotNode}
        action={action}
        onConfig={onConfig}
        issues={[
          {
            code: "LabelInvalid",
            message: "Label is invalid",
            nodeId: screenshotNode.id,
            field: "config.label",
          },
          { code: "NodeInvalid", message: "Node is invalid", nodeId: screenshotNode.id },
        ]}
      />,
    );
    expect(screen.getByLabelText("Nhãn")).toHaveAttribute("maxlength", "96");
    expect(screen.getByLabelText("Định dạng")).toHaveValue("jpeg");
    expect(screen.getByText("Label is invalid")).toHaveAttribute("role", "alert");
    expect(screen.getByText("Node is invalid")).toHaveAttribute("role", "alert");
    fireEvent.change(screen.getByLabelText("Nhãn"), { target: { value: "x".repeat(97) } });
    expect(onConfig).not.toHaveBeenCalled();
  });
});

describe("FlowCoordinatePicker", () => {
  it("stores click coordinates in original frame space", () => {
    const onPick = vi.fn();
    const frame: FlowCoordinateFrame = {
      jpegBase64: "fixture-jpeg",
      imageWidth: 375,
      imageHeight: 667,
      orientation: "portrait",
      profileId: "11".repeat(32),
    };
    render(<FlowCoordinatePicker frame={frame} onPick={onPick} />);
    const image = screen.getByRole("img", { name: "Device frame" });
    vi.spyOn(image, "getBoundingClientRect").mockReturnValue({
      x: 0,
      y: 0,
      left: 0,
      top: 0,
      right: 750,
      bottom: 667,
      width: 750,
      height: 667,
      toJSON: () => ({}),
    } as DOMRect);
    fireEvent.click(image, { clientX: 375, clientY: 333.5 });
    expect(onPick).toHaveBeenCalledWith({
      x: 187.5,
      y: 333.5,
      imageWidth: 375,
      imageHeight: 667,
      orientation: "portrait",
      profileId: "11".repeat(32),
    });
  });
});

describe("FlowImportDialog", () => {
  it("shows every legacy diagnostic and applies only a clean non-null document", async () => {
    const document = newFlowDocument("Imported");
    const onImport = vi.fn();
    const diagnostics: LegacyImportResult = {
      document,
      diagnostics: [
        { stepIndex: 1, code: "UnsupportedStep", message: "First", field: "action" },
        { stepIndex: 2, code: "InvalidValue", message: "Second", field: null },
      ],
    };
    const importer = vi.fn().mockResolvedValueOnce(diagnostics).mockResolvedValueOnce({
      document,
      diagnostics: [],
    });
    render(<FlowImportDialog onImport={onImport} onClose={vi.fn()} importLegacy={importer} />);
    fireEvent.change(screen.getByLabelText("JSON script cũ"), {
      target: { value: '{"steps":[]}' },
    });
    fireEvent.click(screen.getByRole("button", { name: "Import" }));
    expect(await screen.findByText("UnsupportedStep")).toBeInTheDocument();
    expect(screen.getByText("InvalidValue")).toBeInTheDocument();
    expect(onImport).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Import" }));
    await waitFor(() => expect(onImport).toHaveBeenCalledWith(document));
  });
});

describe("FlowJsonDialog", () => {
  it("rejects malformed JSON and backend-invalid documents before applying", async () => {
    const document = newFlowDocument("JSON");
    const onApply = vi.fn();
    const validate = vi.fn().mockRejectedValueOnce(new Error("BackendValidationFailed"));
    render(
      <FlowJsonDialog
        document={document}
        onApply={onApply}
        onClose={vi.fn()}
        validate={validate}
      />,
    );
    const editor = screen.getByLabelText("JSON tài liệu");
    fireEvent.change(editor, { target: { value: "{" } });
    fireEvent.click(screen.getByRole("button", { name: "Validate and apply" }));
    expect(await screen.findByRole("alert")).not.toHaveTextContent(/^$/);
    expect(validate).not.toHaveBeenCalled();
    expect(onApply).not.toHaveBeenCalled();

    fireEvent.change(editor, { target: { value: JSON.stringify(document) } });
    fireEvent.click(screen.getByRole("button", { name: "Validate and apply" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("BackendValidationFailed");
    expect(validate).toHaveBeenCalledWith(document);
    expect(onApply).not.toHaveBeenCalled();
  });

  it("applies validated JSON and displays exactly the backend export string", async () => {
    const document = newFlowDocument("JSON");
    const onApply = vi.fn();
    const validate = vi.fn().mockResolvedValue(compiled(document));
    const exported = '{"backend":"canonical"}';
    const exportFlow = vi.fn().mockResolvedValue(exported);
    render(
      <FlowJsonDialog
        document={document}
        onApply={onApply}
        onClose={vi.fn()}
        validate={validate}
        exportFlow={exportFlow}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Validate and apply" }));
    await waitFor(() => expect(onApply).toHaveBeenCalledWith(document));

    fireEvent.click(screen.getByRole("button", { name: "Load saved export" }));
    await waitFor(() => expect(screen.getByLabelText("JSON tài liệu")).toHaveValue(exported));
    expect(exportFlow).toHaveBeenCalledWith(document.id, document.revision);
  });

  it("rejects JSON larger than one MiB without calling backend validation", async () => {
    const document = newFlowDocument("Large JSON");
    const validate = vi.fn();
    const onApply = vi.fn();
    render(
      <FlowJsonDialog
        document={document}
        onApply={onApply}
        onClose={vi.fn()}
        validate={validate}
      />,
    );
    fireEvent.change(screen.getByLabelText("JSON tài liệu"), {
      target: { value: " ".repeat(1_048_577) },
    });
    fireEvent.click(screen.getByRole("button", { name: "Validate and apply" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("FlowImportTooLarge");
    expect(validate).not.toHaveBeenCalled();
    expect(onApply).not.toHaveBeenCalled();
  });
});
