import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  ActionDefinition,
  ActionKind,
  CompiledRevision,
  DeviceInfo,
  FlowDocumentV2,
  FlowNode,
  FlowRevisionRecord,
  FlowRunRecord,
  FlowSummary,
  JsonValue,
} from "../../types";
import { FlowWorkspace } from "./FlowWorkspace";

const api = vi.hoisted(() => ({
  flowActionCatalog: vi.fn(),
  flowArchive: vi.fn(),
  flowCancelRun: vi.fn(),
  flowCoordinateFrame: vi.fn(),
  flowExport: vi.fn(),
  flowGet: vi.fn(),
  flowGetRun: vi.fn(),
  flowImportLegacy: vi.fn(),
  flowList: vi.fn(),
  flowListRuns: vi.fn(),
  flowReadArtifact: vi.fn(),
  flowRetryAttempt: vi.fn(),
  flowRun: vi.fn(),
  flowSaveRevision: vi.fn(),
  flowValidate: vi.fn(),
  listenRiviuEvents: vi.fn(),
}));

let riviuEventHandler: ((payload: unknown) => void) | undefined;

vi.mock("../../api", () => api);

vi.mock("./FlowCanvas", () => ({
  FlowCanvas: ({
    document,
    catalog,
    onAppendNode,
  }: {
    document: FlowDocumentV2;
    catalog: ActionDefinition[];
    onAppendNode: (node: FlowNode) => void;
  }) => {
    const waitEnabled = catalog.some(
      (action) => action.kind === "wait" && action.disabledReason === null,
    );
    return (
      <section data-testid="flow-canvas">
        <output data-testid="canvas-kinds">
          {document.nodes.map((node) => node.kind).join(",")}
        </output>
        <button
          type="button"
          disabled={!waitEnabled}
          onClick={() => onAppendNode({
            id: "wait-fixture",
            kind: "wait",
            position: { x: 160, y: 160 },
            config: { durationMs: 1_000 },
            postcondition: null,
          })}
        >
          Append Wait fixture
        </button>
      </section>
    );
  },
}));

const savedDocument: FlowDocumentV2 = {
  schemaVersion: 2,
  id: "flow-saved",
  name: "Saved flow",
  revision: 2,
  entryNodeId: "start-saved",
  nodes: [
    { id: "start-saved", kind: "start", position: { x: 0, y: 80 }, config: {} },
    { id: "end-saved", kind: "end", position: { x: 320, y: 80 }, config: {} },
  ],
  edges: [
    {
      id: "edge-saved",
      sourceNodeId: "start-saved",
      sourcePort: "flow",
      targetNodeId: "end-saved",
      targetPort: "flow",
    },
  ],
  viewport: { x: 0, y: 0, zoom: 1 },
};

function action(
  kind: ActionKind,
  label: string,
  category: ActionDefinition["category"],
  configSchema: JsonValue,
  disabledReason: string | null = null,
): ActionDefinition {
  return {
    kind,
    schemaVersion: 1,
    label,
    disabledReason,
    category,
    configSchema,
    inputPorts: [],
    outputPorts: [],
    requiredCapabilities: [],
    resourceClass: kind === "wait" ? "pureDesktop" : "uiSession",
    sideEffectClass: "none",
    evidenceRequirement: "none",
    allowedEvidence: [],
    qualifiedDetectorIds: [],
    reconciliationPolicy: "none",
    defaultTimeoutMs: 1_000,
    retryPolicy: "beforeDispatchOnly",
  };
}

const catalog: ActionDefinition[] = [
  action("start", "Start", "control", { type: "object", properties: {} }),
  action("end", "End", "control", { type: "object", properties: {} }),
  action(
    "launchApp",
    "Launch App",
    "app",
    { type: "object", properties: { bundleId: { type: "string" } } },
    "Requires qualified device",
  ),
  action(
    "wait",
    "Wait",
    "timing",
    {
      type: "object",
      properties: { durationMs: { type: "integer", minimum: 1, maximum: 60_000 } },
    },
  ),
  action("rawHttp", "Raw HTTP", "app", null),
  // Filed under `control` by the backend, exactly as `catalog.rs::category` files it.
  // That is the whole point of the fixture: the palette must reach it there.
  action("ifVision", "If Vision", "control", {
    type: "object",
    properties: { templatePngBase64: { type: "string" } },
  }),
];

function compiled(document: FlowDocumentV2): CompiledRevision {
  return {
    plan: {
      schemaVersion: 2,
      flowId: document.id,
      revision: document.revision,
      nodes: {},
      executionOrder: document.nodes.map((node) => node.id),
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
    canonicalJson: JSON.stringify(document),
    sha256: "11".repeat(32),
  };
}

function revisionRecord(document: FlowDocumentV2): FlowRevisionRecord {
  return {
    document: structuredClone(document),
    compiledPlan: compiled(document).plan,
    planHash: "11".repeat(32),
    createdAt: "2026-07-31T01:00:00.000Z",
  };
}

function savedSummaryForTest(document: FlowDocumentV2): FlowSummary {
  return {
    id: document.id,
    name: document.name,
    latestRevision: document.revision,
    archived: false,
    updatedAt: "2026-07-31T01:00:00.000Z",
  };
}

const device: DeviceInfo = {
  udid: "device-a",
  name: "Device A",
  model: "iPhone fixture",
  platform: "ios",
  osVersion: "16.0",
  connection: "mock",
  status: "ready",
  wdaReady: true,
};

function runRecord(document: FlowDocumentV2): FlowRunRecord {
  return {
    id: "run-a",
    flowId: document.id,
    flowRevision: document.revision,
    planSha256: "11".repeat(32),
    selection: {
      requested: { mode: "selected", udids: [device.udid] },
      targetUdids: [device.udid],
    },
    state: "queued",
    eventRevision: 1,
    error: null,
    createdAt: "2026-07-31T01:00:00.000Z",
    updatedAt: "2026-07-31T01:00:00.000Z",
  };
}

beforeEach(() => {
  localStorage.clear();
  riviuEventHandler = undefined;
  for (const mock of Object.values(api)) mock.mockReset();
  api.flowActionCatalog.mockResolvedValue(catalog);
  api.flowList.mockResolvedValue([
    {
      id: savedDocument.id,
      name: savedDocument.name,
      latestRevision: savedDocument.revision,
      archived: false,
      updatedAt: "2026-07-31T01:00:00.000Z",
    },
  ]);
  api.flowListRuns.mockResolvedValue([]);
  api.flowGet.mockResolvedValue(revisionRecord(savedDocument));
  api.flowValidate.mockImplementation(async (document: FlowDocumentV2) => compiled(document));
  api.flowSaveRevision.mockImplementation(async (document: FlowDocumentV2) => {
    const saved = { ...structuredClone(document), revision: document.revision + 1 };
    return revisionRecord(saved);
  });
  api.flowRun.mockImplementation(async (id: string, revision: number) =>
    runRecord({ ...savedDocument, id, revision }));
  api.flowGetRun.mockResolvedValue(null);
  api.listenRiviuEvents.mockImplementation(async (handler: (payload: unknown) => void) => {
    riviuEventHandler = handler;
    return vi.fn();
  });
});

afterEach(() => {
  cleanup();
  localStorage.clear();
});

async function renderReadyWorkspace(onDirtyChange = vi.fn()) {
  render(
    <FlowWorkspace
      devices={[device]}
      selectedUdids={[device.udid]}
      onDirtyChange={onDirtyChange}
    />,
  );
  await screen.findByDisplayValue("Saved flow");
  await waitFor(() => expect(api.flowValidate).toHaveBeenCalled());
  // The load-tolerant `waitFor` default lives in `src/test/setup.ts`; see the comment there
  // for why 1 s was a load threshold rather than a behaviour one.
  await waitFor(() => expect(screen.getByRole("button", { name: "Chạy Flow" })).toBeEnabled());
  return onDirtyChange;
}

describe("FlowWorkspace startup", () => {
  it("loads the saved revision, keeps disabled catalog reasons, and omits raw actions", async () => {
    await renderReadyWorkspace();

    expect(api.flowActionCatalog).toHaveBeenCalledTimes(1);
    expect(api.flowList).toHaveBeenCalledWith();
    expect(api.flowListRuns).toHaveBeenCalledWith(100);
    expect(api.flowGet).toHaveBeenCalledWith(savedDocument.id);
    const disabledLaunch = screen.getByRole("button", { name: "Mở ứng dụng" });
    expect(disabledLaunch).toBeDisabled();
    expect(disabledLaunch).toHaveAttribute("title", "Requires qualified device");
    expect(screen.queryByRole("button", { name: "Raw HTTP" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Lưu bản" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Chạy Flow" })).toBeEnabled();
  });

  it("offers the one branching action, and still not the two nodes the canvas owns", async () => {
    // The palette used to drop the entire `control` category to keep Start and End out.
    // `ifVision` is filed there too, so the only conditional action in the product could
    // not be placed on a canvas -- while its two ports, its config default and its
    // compiler support were all finished. Nothing said so; it simply was not in the list.
    await renderReadyWorkspace();

    expect(screen.getByRole("button", { name: "Nếu thấy ảnh" })).toBeEnabled();
    // Start and End are created with the document, so offering them would let an operator
    // drop a second one. Kept out by kind now, not by category.
    expect(screen.queryByRole("button", { name: "Bắt đầu" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Kết thúc" })).not.toBeInTheDocument();
  });

  it("leaves loading state and reports a saved-revision fetch failure", async () => {
    api.flowGet.mockRejectedValueOnce(new Error("revision read failed"));

    render(
      <FlowWorkspace
        devices={[device]}
        selectedUdids={[device.udid]}
        onDirtyChange={vi.fn()}
      />,
    );

    expect(await screen.findByRole("alert")).toHaveTextContent("revision read failed");
    await waitFor(() => expect(
      screen.getByRole("region", { name: "Không gian Flow" }),
    ).toHaveAttribute("data-loading", "false"));
  });
});

describe("FlowWorkspace editing", () => {
  it("reloads a clean document after a Flow invalidation", async () => {
    await renderReadyWorkspace();
    await waitFor(() => expect(riviuEventHandler).toBeDefined());
    const updated = { ...structuredClone(savedDocument), name: "Externally saved", revision: 3 };
    api.flowList.mockResolvedValue([savedSummaryForTest(updated)]);
    api.flowGet.mockResolvedValue(revisionRecord(updated));

    act(() => riviuEventHandler?.({
      type: "flowUpdated",
      flowId: savedDocument.id,
      revision: 1,
    }));

    expect(await screen.findByDisplayValue("Externally saved")).toBeInTheDocument();
    expect(api.flowGet).toHaveBeenLastCalledWith(savedDocument.id);
  });

  it("refreshes the list but preserves a dirty draft after a Flow invalidation", async () => {
    await renderReadyWorkspace();
    fireEvent.click(screen.getByRole("button", { name: "Append Wait fixture" }));
    await waitFor(() => expect(riviuEventHandler).toBeDefined());
    const updated = { ...structuredClone(savedDocument), name: "Externally saved", revision: 3 };
    api.flowList.mockResolvedValue([savedSummaryForTest(updated)]);
    api.flowGet.mockResolvedValue(revisionRecord(updated));
    const readsBeforeEvent = api.flowGet.mock.calls.length;

    act(() => riviuEventHandler?.({
      type: "flowUpdated",
      flowId: savedDocument.id,
      revision: 2,
    }));

    await waitFor(() => expect(api.flowList).toHaveBeenCalledTimes(2));
    expect(api.flowGet).toHaveBeenCalledTimes(readsBeforeEvent);
    expect(screen.getByDisplayValue("Saved flow")).toBeInTheDocument();
    expect(screen.getByTestId("canvas-kinds")).toHaveTextContent("start,end,wait");
  });

  it("makes an appended action reducer-visible and restores the document through Undo", async () => {
    const onDirtyChange = await renderReadyWorkspace();
    fireEvent.click(screen.getByRole("button", { name: "Append Wait fixture" }));

    expect(screen.getByTestId("canvas-kinds")).toHaveTextContent("start,end,wait");
    expect(screen.getByRole("button", { name: "Hoàn tác" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Chạy Flow" })).toBeDisabled();
    await waitFor(() => expect(onDirtyChange).toHaveBeenLastCalledWith(true));

    fireEvent.click(screen.getByRole("button", { name: "Hoàn tác" }));
    expect(screen.getByTestId("canvas-kinds")).toHaveTextContent("start,end");
    expect(screen.getByTestId("canvas-kinds")).not.toHaveTextContent("wait");
    expect(screen.getByRole("button", { name: "Làm lại" })).toBeEnabled();
  });

  it("enables Save only after current validation, then enables Run after a clean saved revision", async () => {
    await renderReadyWorkspace();
    fireEvent.click(screen.getByRole("button", { name: "Append Wait fixture" }));

    const save = screen.getByRole("button", { name: "Lưu bản" });
    const run = screen.getByRole("button", { name: "Chạy Flow" });
    expect(save).toBeDisabled();
    expect(run).toBeDisabled();
    await waitFor(() => expect(save).toBeEnabled());

    fireEvent.click(save);
    await waitFor(() => expect(api.flowSaveRevision).toHaveBeenCalledTimes(1));
    expect(api.flowSaveRevision.mock.calls[0][1]).toBe(2);
    await waitFor(() => expect(run).toBeEnabled());
    expect(save).toBeDisabled();

    fireEvent.click(run);
    fireEvent.click(screen.getByRole("button", { name: "Chạy trên thiết bị" }));
    await waitFor(() => expect(api.flowRun).toHaveBeenCalledWith(
      savedDocument.id,
      3,
      { mode: "selected", udids: [device.udid] },
    ));
  });
});
