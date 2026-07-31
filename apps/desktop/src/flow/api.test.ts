import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { FlowDocumentV2 } from "../types";
import {
  flowActionCatalog,
  flowArchive,
  flowCancelRun,
  flowCoordinateFrame,
  flowExport,
  flowGet,
  flowGetRun,
  flowImportLegacy,
  flowList,
  flowListRuns,
  flowReadArtifact,
  flowRetryAttempt,
  flowRun,
  flowSaveRevision,
  flowValidate,
} from "../api";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const document: FlowDocumentV2 = {
  schemaVersion: 2,
  id: "flow-a",
  name: "Fixture",
  revision: 5,
  entryNodeId: "start-a",
  nodes: [
    { id: "start-a", kind: "start", position: { x: 0, y: 80 }, config: {} },
    { id: "end-a", kind: "end", position: { x: 320, y: 80 }, config: {} },
  ],
  edges: [
    {
      id: "edge-a",
      sourceNodeId: "start-a",
      sourcePort: "flow",
      targetNodeId: "end-a",
      targetPort: "flow",
    },
  ],
  viewport: { x: 0, y: 0, zoom: 1 },
};

beforeEach(() => {
  vi.mocked(invoke).mockReset().mockResolvedValue(undefined);
});

describe("Flow API client", () => {
  it("uses exact command names and camelCase arguments", async () => {
    await flowActionCatalog();
    await flowList(true);
    await flowGet("flow-a", 3);
    await flowValidate(document);
    await flowSaveRevision(document, 4);
    await flowArchive("flow-a");
    await flowImportLegacy("{\"version\":1}");
    await flowExport("flow-a", 5);
    await flowRun("flow-a", 5, { mode: "selected", udids: ["device-a"] });
    await flowCancelRun("run-a");
    await flowRetryAttempt("attempt-a");
    await flowListRuns(25);
    await flowGetRun("run-a");
    await flowCoordinateFrame("device-a", "com.apple.Preferences");
    await flowReadArtifact("artifact-a");

    expect(vi.mocked(invoke).mock.calls).toEqual([
      ["flow_action_catalog"],
      ["flow_list", { includeArchived: true }],
      ["flow_get", { id: "flow-a", revision: 3 }],
      ["flow_validate", { document }],
      ["flow_save_revision", { document, expectedRevision: 4 }],
      ["flow_archive", { id: "flow-a" }],
      ["flow_import_legacy", { scriptJson: "{\"version\":1}" }],
      ["flow_export", { id: "flow-a", revision: 5 }],
      [
        "flow_run",
        {
          id: "flow-a",
          revision: 5,
          selection: { mode: "selected", udids: ["device-a"] },
        },
      ],
      ["flow_cancel_run", { runId: "run-a" }],
      ["flow_retry_attempt", { attemptId: "attempt-a" }],
      ["flow_list_runs", { limit: 25 }],
      ["flow_get_run", { runId: "run-a" }],
      ["flow_coordinate_frame", { udid: "device-a", bundleId: "com.apple.Preferences" }],
      ["flow_read_artifact", { artifactId: "artifact-a" }],
    ]);
  });

  it("serializes absent optional revisions as null", async () => {
    await flowGet("flow-a");
    await flowExport("flow-a");
    expect(invoke).toHaveBeenNthCalledWith(1, "flow_get", { id: "flow-a", revision: null });
    expect(invoke).toHaveBeenNthCalledWith(2, "flow_export", {
      id: "flow-a",
      revision: null,
    });
  });
});
