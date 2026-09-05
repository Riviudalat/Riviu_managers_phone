import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  ActionDefinition,
  CompiledRevision,
  DeviceInfo,
  FlowDocumentV2,
  FlowRevisionRecord,
  FlowSummary,
} from "../../types";
import { FlowWorkspace } from "./FlowWorkspace";

/**
 * Four ways the workspace could lose or misreport the operator's unsaved work.
 *
 * Every one of them is a command the toolbar offers with no mention of the draft: Archive cleared
 * it, Import replaced it, Export quietly shipped the *stored* revision instead, and the compile
 * preview called an unchecked document invalid. `New`, `Duplicate` and picking another flow all ask
 * before discarding; these did not, and nothing in the suite noticed because every existing case
 * drives them on a clean document.
 */

const api = vi.hoisted(() => ({
  flowActionCatalog: vi.fn(),
  flowArchive: vi.fn(),
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
  flowCancelRun: vi.fn(),
  flowSaveRevision: vi.fn(),
  flowValidate: vi.fn(),
  listenRiviuEvents: vi.fn(),
}));

vi.mock("../../api", () => api);

const confirmed = vi.hoisted(() => ({ requestConfirm: vi.fn(), requestSaveChanges: vi.fn() }));
vi.mock("../../confirmStore", () => confirmed);

/**
 * The canvas is replaced: React Flow needs a measured viewport, and these cases are about the
 * toolbar's commands, not about hit testing. The stand-in exposes one button that appends an action,
 * which is the shortest route to a dirty document.
 */
vi.mock("./FlowCanvas", () => ({
  FlowCanvas: ({
    document,
    onAppendNode,
  }: {
    document: FlowDocumentV2;
    onAppendNode: (node: {
      id: string;
      kind: string;
      position: { x: number; y: number };
      config: Record<string, never>;
    }) => void;
  }) => (
    <div data-testid="flow-canvas">
      <output data-testid="canvas-kinds">
        {document.nodes.map((node) => node.kind).join(",")}
      </output>
      <button
        type="button"
        onClick={() =>
          onAppendNode({
            id: "appended-node",
            kind: "home",
            position: { x: 10, y: 10 },
            config: {},
          })
        }
      >
        Append fixture
      </button>
    </div>
  ),
}));

const device: DeviceInfo = {
  udid: "MOCK-01",
  name: "Fixture phone",
  platform: "android",
  status: "ready",
} as unknown as DeviceInfo;

const savedDocument: FlowDocumentV2 = {
  schemaVersion: 2,
  id: "33333333-3333-4333-8333-333333333333",
  name: "Saved flow",
  revision: 2,
  entryNodeId: "start-node",
  nodes: [
    { id: "start-node", kind: "start", position: { x: 0, y: 0 }, config: {} },
    { id: "end-node", kind: "end", position: { x: 300, y: 0 }, config: {} },
  ],
  edges: [
    {
      id: "edge-1",
      sourceNodeId: "start-node",
      sourcePort: "flow",
      targetNodeId: "end-node",
      targetPort: "flow",
    },
  ],
  viewport: { x: 0, y: 0, zoom: 1 },
};

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
    sha256: "44".repeat(32),
  };
}

function revisionRecord(document: FlowDocumentV2): FlowRevisionRecord {
  return {
    document: structuredClone(document),
    compiledPlan: compiled(document).plan,
    planHash: "44".repeat(32),
    createdAt: "2026-08-28T00:00:00.000Z",
  };
}

const summary: FlowSummary = {
  id: savedDocument.id,
  name: savedDocument.name,
  revision: savedDocument.revision,
  archived: false,
  updatedAt: "2026-08-28T00:00:00.000Z",
} as unknown as FlowSummary;

const catalog: ActionDefinition[] = [
  {
    kind: "home",
    schemaVersion: 1,
    label: "Home",
    disabledReason: null,
    category: "control",
    configSchema: { type: "object", properties: {} },
    inputPorts: [],
    outputPorts: [],
    requiredCapabilities: [],
    resourceClass: "uiSession",
    sideEffectClass: "ambiguousUi",
    evidenceRequirement: "none",
    allowedEvidence: [],
    qualifiedDetectorIds: [],
    reconciliationPolicy: "none",
    defaultTimeoutMs: 5_000,
    retryPolicy: "beforeDispatchOnly",
  },
];

beforeEach(() => {
  localStorage.clear();
  for (const mock of Object.values(api)) mock.mockReset();
  confirmed.requestConfirm.mockReset().mockResolvedValue(true);
  confirmed.requestSaveChanges.mockReset().mockResolvedValue("discard");
  api.flowActionCatalog.mockResolvedValue(catalog);
  api.flowList.mockResolvedValue([summary]);
  api.flowListRuns.mockResolvedValue([]);
  api.flowGet.mockResolvedValue(revisionRecord(savedDocument));
  api.flowGetRun.mockResolvedValue(null);
  api.flowValidate.mockImplementation(async (document: FlowDocumentV2) => compiled(document));
  api.flowArchive.mockResolvedValue(undefined);
  api.flowExport.mockResolvedValue("{}");
  api.listenRiviuEvents.mockResolvedValue(vi.fn());
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

async function openDirtyWorkspace() {
  render(<FlowWorkspace devices={[device]} selectedUdids={[]} onDirtyChange={vi.fn()} />);
  await waitFor(() =>
    expect(screen.getByRole("region", { name: "Không gian Flow" })).toHaveAttribute(
      "data-loading",
      "false",
    ),
  );
  await waitFor(() => expect(screen.getByDisplayValue("Saved flow")).toBeInTheDocument());
  fireEvent.click(screen.getByRole("button", { name: "Append fixture" }));
  expect(screen.getByTestId("canvas-kinds")).toHaveTextContent("start,end,home");
  return screen.getByTestId("canvas-kinds");
}

describe("commands that would throw away an unsaved draft", () => {
  it("asks before archiving a flow with unsaved work", async () => {
    // The archive dialog talks about the flow leaving the active list. It never mentioned the
    // draft, and the handler then cleared it and opened another flow.
    await openDirtyWorkspace();
    fireEvent.click(screen.getByRole("button", { name: "Lưu trữ Flow" }));

    await waitFor(() => expect(confirmed.requestConfirm).toHaveBeenCalled());
    expect(confirmed.requestSaveChanges).toHaveBeenCalledWith("Flow thiết bị");
  });

  it("does not archive when the operator keeps the draft", async () => {
    await openDirtyWorkspace();
    confirmed.requestSaveChanges.mockResolvedValue("stay");
    fireEvent.click(screen.getByRole("button", { name: "Lưu trữ Flow" }));

    await waitFor(() => expect(confirmed.requestSaveChanges).toHaveBeenCalledTimes(1));
    expect(api.flowArchive).not.toHaveBeenCalled();
    expect(screen.getByTestId("canvas-kinds")).toHaveTextContent("start,end,home");
  });

  it("asks before opening the import dialog over unsaved work", async () => {
    // A successful import replaces the open document outright, which is the same discard New and
    // Duplicate both ask about.
    await openDirtyWorkspace();
    confirmed.requestSaveChanges.mockResolvedValue("stay");
    fireEvent.click(screen.getByRole("button", { name: "Nhập Flow" }));

    await waitFor(() => expect(confirmed.requestSaveChanges).toHaveBeenCalledTimes(1));
    expect(screen.queryByRole("dialog", { name: "Nhập Flow cũ" })).toBeNull();
  });

  it("refuses to export a dirty flow instead of shipping the stored revision", async () => {
    // The backend exports a saved revision. Offered on a dirty graph it downloaded the stored copy,
    // under the name on screen, and reported success.
    await openDirtyWorkspace();
    const exportButton = screen.getByRole("button", { name: "Xuất Flow" });
    expect(exportButton).toBeDisabled();
    expect(exportButton).toHaveAttribute("title", expect.stringContaining("Lưu bản"));
    fireEvent.click(exportButton);
    expect(api.flowExport).not.toHaveBeenCalled();
  });
});

describe("the compile preview has three answers, not two", () => {
  it("does not call an unchecked document invalid", async () => {
    // Every edit clears the compiled plan before the debounced request goes out, so equating
    // `compiled === null` with invalid announced Invalid over a document nobody had checked yet.
    let release: ((value: CompiledRevision) => void) | undefined;
    api.flowValidate.mockImplementation(
      () => new Promise<CompiledRevision>((resolve) => {
        release = resolve;
      }),
    );
    await openDirtyWorkspace();

    fireEvent.click(screen.getByRole("button", { name: "Kiểm tra Flow" }));
    const preview = screen.getByRole("dialog", { name: "Xem trước biên dịch" });
    await waitFor(() => expect(within(preview).getByText("Đang kiểm…")).toBeInTheDocument());
    expect(within(preview).queryByText("Chưa hợp lệ")).toBeNull();

    // Deliberately not asserting the settled state through this same pending promise: the reducer
    // binds a validation result to the document epoch it was requested for, so a hand-resolved
    // fixture promise is not a faithful stand-in. The settled reading gets its own case below.
    release?.(compiled(savedDocument));
    expect(within(preview).queryByText("Chưa hợp lệ")).toBeNull();
  });

  it("reports a valid document in Vietnamese and keeps the raw context inside details", async () => {
    render(<FlowWorkspace devices={[device]} selectedUdids={[]} onDirtyChange={vi.fn()} />);
    await waitFor(() =>
      expect(screen.getByRole("region", { name: "Không gian Flow" })).toHaveAttribute(
        "data-loading",
        "false",
      ),
    );
    await waitFor(() => expect(api.flowValidate).toHaveBeenCalled());

    fireEvent.click(screen.getByRole("button", { name: "Kiểm tra Flow" }));
    const preview = screen.getByRole("dialog", { name: "Xem trước biên dịch" });
    await waitFor(() => expect(within(preview).getByText("Hợp lệ")).toBeInTheDocument());
    expect(within(preview).queryByText("Valid")).toBeNull();
    expect(within(preview).getByText("Chi tiết kỹ thuật").closest("details")).not.toBeNull();
  });
});
