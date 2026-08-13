import type { Page } from "@playwright/test";

export type MockRunMode = "succeeded" | "uncertainTap" | "runningWait";

export interface TauriMockOptions {
  initialRunMode?: MockRunMode;
}

export interface MockCommandCall {
  command: string;
  args: Record<string, unknown>;
}

export async function installTauriMock(
  page: Page,
  options: TauriMockOptions = {},
): Promise<void> {
  await page.addInitScript((fixtureOptions: TauriMockOptions) => {
    type JsonRecord = Record<string, unknown>;
    type Callback = (event: unknown) => void;
    type Handler = (args: JsonRecord) => unknown | Promise<unknown>;

    const PERSISTENCE_KEY = "riviu.e2e.tauri-state.v1";
    const AT = "2026-07-30T00:00:00Z";
    const FLOW_ID = "00000000-0000-0000-0000-000000000100";
    const START_ID = "00000000-0000-0000-0000-000000000101";
    const WAIT_ID = "00000000-0000-0000-0000-000000000102";
    const END_ID = "00000000-0000-0000-0000-000000000103";
    const TAP_ID = "00000000-0000-0000-0000-000000000104";
    const EDGE_A_ID = "00000000-0000-0000-0000-000000000201";
    const EDGE_B_ID = "00000000-0000-0000-0000-000000000202";
    const EDGE_C_ID = "00000000-0000-0000-0000-000000000203";
    const RUN_ID = "00000000-0000-0000-0000-000000000301";
    const JPEG_BASE64 =
      "/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAP//////////////////////////////////////////////////////////////////////////////////////2wBDAf//////////////////////////////////////////////////////////////////////////////////////wAARCAABAAEDASIAAhEBAxEB/8QAFQABAQAAAAAAAAAAAAAAAAAAAAf/xAAUEAEAAAAAAAAAAAAAAAAAAAAA/9oADAMBAAIQAxAAAAF//8QAFBABAAAAAAAAAAAAAAAAAAAAAP/aAAgBAQABBQJ//8QAFBEBAAAAAAAAAAAAAAAAAAAAAP/aAAgBAwEBPwF//8QAFBEBAAAAAAAAAAAAAAAAAAAAAP/aAAgBAgEBPwF//8QAFBABAAAAAAAAAAAAAAAAAAAAAP/aAAgBAQAGPwJ//8QAFBABAAAAAAAAAAAAAAAAAAAAAP/aAAgBAQABPyF//9oADAMBAAIAAwAAABD/xAAUEQEAAAAAAAAAAAAAAAAAAAAA/9oACAEDAQE/EH//xAAUEQEAAAAAAAAAAAAAAAAAAAAA/9oACAECAQE/EH//xAAUEAEAAAAAAAAAAAAAAAAAAAAA/9oACAEBAAE/EH//2Q==";

    const callbacks = new Map<number, Callback>();
    const listeners = new Map<number, number>();
    const commandHandlers = new Map<string, Handler>();
    const commandCalls: MockCommandCall[] = [];
    let nextCallbackId = 1;
    let nextListenerId = 1;
    let volatileRunMode: MockRunMode = fixtureOptions.initialRunMode ?? "succeeded";

    const clone = <T>(value: T): T => structuredClone(value);
    const uuid = (value: number): string =>
      `00000000-0000-0000-0000-${value.toString().padStart(12, "0")}`;

    const fixtureDocument = () => ({
      schemaVersion: 2,
      id: FLOW_ID,
      name: "Fixture flow",
      revision: 1,
      entryNodeId: START_ID,
      nodes: [
        { id: START_ID, kind: "start", position: { x: 0, y: 80 }, config: {}, postcondition: null },
        {
          id: WAIT_ID,
          kind: "wait",
          position: { x: 240, y: 80 },
          config: { durationMs: 250 },
          postcondition: null,
        },
        {
          id: TAP_ID,
          kind: "tap",
          position: { x: 480, y: 80 },
          config: { accessibilityId: "like-button" },
          postcondition: { kind: "frameDigestChanged", minimumDistance: 8 },
        },
        { id: END_ID, kind: "end", position: { x: 720, y: 80 }, config: {}, postcondition: null },
      ],
      edges: [
        {
          id: EDGE_A_ID,
          sourceNodeId: START_ID,
          sourcePort: "flow",
          targetNodeId: WAIT_ID,
          targetPort: "flow",
        },
        {
          id: EDGE_B_ID,
          sourceNodeId: WAIT_ID,
          sourcePort: "flow",
          targetNodeId: TAP_ID,
          targetPort: "flow",
        },
        {
          id: EDGE_C_ID,
          sourceNodeId: TAP_ID,
          sourcePort: "flow",
          targetNodeId: END_ID,
          targetPort: "flow",
        },
      ],
      viewport: { x: 0, y: 0, zoom: 0.85 },
    });

    function compiledFor(document: JsonRecord) {
      const nodes = (document.nodes as JsonRecord[]).reduce<JsonRecord>((result, node) => {
        result[node.id as string] = {
          id: node.id,
          kind: node.kind,
          config: { kind: node.kind === "wait" ? "wait" : "empty", ...(node.config as JsonRecord) },
          postcondition: node.postcondition ?? null,
        };
        return result;
      }, {});
      const executionOrder = (document.nodes as JsonRecord[]).map((node) => node.id as string);
      return {
        plan: {
          schemaVersion: 2,
          flowId: document.id,
          revision: document.revision,
          nodes,
          executionOrder,
          contextPlan: {
            requiresExclusive: false,
            requiresUiSession: false,
            requiresStream: false,
            requiresFreshTextSession: false,
            initialBundleId: null,
          },
          actionDefinitionVersions: Object.fromEntries(
            (document.nodes as JsonRecord[]).map((node) => [node.kind as string, 1]),
          ),
          requiredCapabilities: [],
        },
        canonicalJson: JSON.stringify(document),
        sha256: "44".repeat(32),
      };
    }

    function revisionFor(document: JsonRecord) {
      const compiled = compiledFor(document);
      return {
        document: clone(document),
        compiledPlan: compiled.plan,
        planHash: compiled.sha256,
        createdAt: AT,
      };
    }

    interface BackendState {
      revisions: JsonRecord[];
      runs: JsonRecord[];
      details: Record<string, JsonRecord>;
      nextRunNumber: number;
    }

    function initialState(): BackendState {
      const document = fixtureDocument();
      const revision = revisionFor(document);
      return {
        revisions: [revision],
        runs: [],
        details: {},
        nextRunNumber: 302,
      };
    }

    function readState(): BackendState {
      try {
        const raw = localStorage.getItem(PERSISTENCE_KEY);
        return raw ? (JSON.parse(raw) as BackendState) : initialState();
      } catch {
        return initialState();
      }
    }

    let state = readState();
    function persist(): void {
      localStorage.setItem(PERSISTENCE_KEY, JSON.stringify(state));
    }

    function transformCallback(callback: Callback, once = false): number {
      const callbackId = nextCallbackId++;
      callbacks.set(callbackId, (event) => {
        if (once) callbacks.delete(callbackId);
        callback(event);
      });
      return callbackId;
    }

    function unregisterCallback(callbackId: number): void {
      callbacks.delete(callbackId);
    }

    function unregisterListener(listenerId: number): void {
      const callbackId = listeners.get(listenerId);
      listeners.delete(listenerId);
      if (callbackId !== undefined) callbacks.delete(callbackId);
    }

    function emit(payload: unknown): void {
      for (const handlerId of [...listeners.values()]) {
        callbacks.get(handlerId)?.({ id: 1, event: "riviu://event", payload });
      }
    }

    const devices = [
      {
        udid: "MOCK-IPHONE-01",
        name: "Fixture iPhone 01",
        model: "iPhone10,1",
        platform: "ios",
        osVersion: "16.7.15",
        connection: "mock",
        status: "ready",
        battery: 82,
        wdaReady: true,
        tileStreamState: "parked",
      },
      {
        udid: "MOCK-IPHONE-02",
        name: "Fixture iPhone 02",
        model: "iPhone10,1",
        platform: "ios",
        osVersion: "16.7.15",
        connection: "mock",
        status: "ready",
        battery: 76,
        wdaReady: true,
        tileStreamState: "parked",
      },
    ];

    const port = (required = true) => [{ name: "flow", valueType: "flow", required }];
    const definition = (
      kind: string,
      label: string,
      category: string,
      configSchema: JsonRecord,
      options: JsonRecord = {},
    ) => ({
      kind,
      schemaVersion: 1,
      label,
      disabledReason: null,
      category,
      configSchema,
      inputPorts: kind === "start" ? [] : port(),
      outputPorts: kind === "end" ? [] : port(),
      requiredCapabilities: [],
      resourceClass: "pureDesktop",
      sideEffectClass: "none",
      evidenceRequirement: "none",
      allowedEvidence: [],
      qualifiedDetectorIds: [],
      reconciliationPolicy: "none",
      defaultTimeoutMs: 60_000,
      retryPolicy: "never",
      ...options,
    });
    const coordinateSchema = {
      type: "object",
      properties: {
        x: { type: "number", title: "X" },
        y: { type: "number", title: "Y" },
        imageWidth: { type: "integer", minimum: 1 },
        imageHeight: { type: "integer", minimum: 1 },
        orientation: {
          type: "string",
          enum: ["portrait", "portraitUpsideDown", "landscapeLeft", "landscapeRight"],
        },
        profileId: { type: "string" },
      },
    };
    const catalog = [
      definition("start", "Start", "control", { type: "object", properties: {} }),
      definition("end", "End", "control", { type: "object", properties: {} }),
      definition("launchApp", "Launch App", "app", {
        type: "object",
        properties: { bundleId: { type: "string", title: "Bundle ID", minLength: 1 } },
      }, {
        resourceClass: "bridge",
        sideEffectClass: "idempotentSet",
        evidenceRequirement: "activeApp",
        allowedEvidence: ["activeAppEquals"],
        reconciliationPolicy: "readActiveApp",
        retryPolicy: "idempotentAfterRead",
      }),
      definition("terminateApp", "Terminate App", "app", {
        type: "object",
        properties: { bundleId: { type: "string", title: "Bundle ID", minLength: 1 } },
      }, {
        resourceClass: "bridge",
        sideEffectClass: "idempotentSet",
        evidenceRequirement: "process",
        allowedEvidence: ["processAbsent"],
        reconciliationPolicy: "readProcess",
        retryPolicy: "idempotentAfterRead",
      }),
      definition("home", "Home", "app", { type: "object", properties: {} }, {
        resourceClass: "uiSession",
        sideEffectClass: "idempotentSet",
        evidenceRequirement: "activeApp",
        allowedEvidence: ["activeAppEquals"],
        reconciliationPolicy: "readActiveApp",
      }),
      definition("wait", "Wait", "timing", {
        type: "object",
        properties: {
          // No `title`: the real catalog ships none, so the inspector's own
          // label map supplies it. Inventing one here made the e2e exercise a
          // label path production never takes.
          durationMs: {
            type: "integer",
            minimum: 1,
            maximum: 60_000,
          },
        },
      }),
      definition("tap", "Tap", "input", {
        type: "object",
        properties: {
          accessibilityId: { type: "string", title: "Accessibility ID" },
          point: coordinateSchema,
        },
      }, {
        resourceClass: "uiWithStream",
        sideEffectClass: "ambiguousUi",
        evidenceRequirement: "frame",
        allowedEvidence: ["frameDigestChanged", "frameRegionChanged", "accessibilityVisible"],
        reconciliationPolicy: "readFrame",
        retryPolicy: "beforeDispatchOnly",
      }),
      definition("swipe", "Swipe", "input", {
        type: "object",
        properties: {
          from: coordinateSchema,
          to: coordinateSchema,
          durationMs: { type: "integer", minimum: 1, maximum: 5_000 },
        },
      }, {
        resourceClass: "uiWithStream",
        sideEffectClass: "ambiguousUi",
        evidenceRequirement: "frame",
        allowedEvidence: ["frameDigestChanged", "frameRegionChanged"],
        reconciliationPolicy: "readFrame",
        retryPolicy: "beforeDispatchOnly",
      }),
      definition("typeText", "Type Text", "input", {
        type: "object",
        properties: {
          text: { type: "string" },
          readBackLocator: {
            type: "object",
            properties: {
              strategy: { type: "string", enum: ["accessibilityId", "className"] },
              value: { type: "string" },
            },
          },
        },
      }, {
        resourceClass: "uiSession",
        sideEffectClass: "ambiguousUi",
        evidenceRequirement: "textOrQualifiedFrame",
        allowedEvidence: ["textReadBackEquals"],
        reconciliationPolicy: "readText",
      }),
      definition("screenshot", "Screenshot", "evidence", {
        type: "object",
        properties: {
          label: { type: "string" },
          format: { type: "string", enum: ["jpeg"] },
        },
      }, {
        resourceClass: "uiWithStream",
        sideEffectClass: "artifactWrite",
        evidenceRequirement: "artifact",
        allowedEvidence: ["artifactDecodedAndHashed"],
        reconciliationPolicy: "readArtifact",
      }),
      definition("assertVisible", "Assert Visible", "evidence", {
        type: "object",
        properties: { accessibilityId: { type: "string", title: "Accessibility ID" } },
      }, {
        resourceClass: "uiSession",
        allowedEvidence: ["accessibilityVisible"],
      }),
      definition("rawHttp", "Raw HTTP", "app", { type: "object", properties: {} }, {
        disabledReason: "Raw HTTP is not available in Flow V2 release 1.",
      }),
      definition("rawWda", "Raw WDA", "app", { type: "object", properties: {} }, {
        disabledReason: "Raw WDA is not available in Flow V2 release 1.",
      }),
      definition("shell", "Shell", "app", { type: "object", properties: {} }, {
        disabledReason: "Shell is not available in Flow V2 release 1.",
      }),
    ];

    function summaryFor(revision: JsonRecord) {
      const document = revision.document as JsonRecord;
      return {
        id: document.id,
        name: document.name,
        latestRevision: document.revision,
        archived: false,
        updatedAt: revision.createdAt,
      };
    }

    function resolveTargets(selection: JsonRecord): string[] {
      if (selection.mode === "one") return [selection.udid as string];
      if (selection.mode === "selected") return [...(selection.udids as string[])].sort();
      return devices.map((device) => device.udid);
    }

    function createRun(args: JsonRecord, mode: MockRunMode): JsonRecord {
      const runId = state.runs.length === 0 ? RUN_ID : uuid(state.nextRunNumber++);
      const revision = [...state.revisions].reverse().find((candidate) => {
        const document = candidate.document as JsonRecord;
        return document.id === args.id &&
          (args.revision === null || args.revision === undefined || document.revision === args.revision);
      });
      if (!revision) throw { code: "FlowNotFound", message: "Flow revision was not found." };
      const document = revision.document as JsonRecord;
      const selection = args.selection as JsonRecord;
      const targetUdids = resolveTargets(selection);
      const executableNodes = (document.nodes as JsonRecord[]).filter(
        (node) => node.kind !== "start" && node.kind !== "end",
      );
      const aggregateState = mode === "succeeded" ? "succeeded" : mode === "uncertainTap" ? "partial" : "running";
      const deviceState = mode === "succeeded" ? "succeeded" : mode === "uncertainTap" ? "failed" : "running";
      const run = {
        id: runId,
        flowId: document.id,
        flowRevision: document.revision,
        planSha256: revision.planHash,
        selection: { requested: clone(selection), targetUdids },
        state: aggregateState,
        eventRevision: 1,
        error: null,
        createdAt: AT,
        updatedAt: AT,
      };
      const deviceRuns = targetUdids.map((udid, index) => ({
        id: uuid(400 + state.nextRunNumber + index),
        runId,
        udid,
        state: deviceState,
        capabilitySnapshot: null,
        releaseProof: mode === "runningWait" ? null : {
          udid,
          owner: "script",
          hadSession: false,
          hadStream: false,
        },
        error: null,
        startedAt: AT,
        finishedAt: mode === "runningWait" ? null : AT,
      }));
      const attempts = deviceRuns.flatMap((deviceRun, deviceIndex) =>
        executableNodes.map((node, nodeIndex) => {
          const id = uuid(500 + state.nextRunNumber + deviceIndex * 100 + nodeIndex);
          const uncertain = mode === "uncertainTap" && node.kind === "tap";
          const running = mode === "runningWait" && node.kind === "wait";
          const stateName = mode === "succeeded"
            ? "succeeded"
            : uncertain
              ? "uncertain"
              : running
                ? "verifying"
                : mode === "uncertainTap"
                  ? "succeeded"
                  : "queued";
          return {
            id,
            deviceRunId: deviceRun.id,
            nodeId: node.id,
            actionKind: node.kind,
            attemptNo: 1,
            sideEffectClass: node.kind === "tap"
              ? "ambiguousUi"
              : node.kind === "screenshot"
                ? "artifactWrite"
                : "none",
            state: stateName,
            canonicalInput: node.config ?? {},
            evidenceBaseline: node.kind === "tap" ? { digest: "11".repeat(32) } : null,
            evidenceResult: stateName === "succeeded" ? { matched: true } : null,
            retryAllowed: false,
            error: uncertain ? {
              code: "UncertainAfterDispatch",
              message: "Tap outcome could not be reconciled.",
              nodeId: node.id,
              field: null,
              udid: deviceRun.udid,
              attemptId: id,
            } : null,
            startedAt: stateName === "queued" ? null : AT,
            updatedAt: AT,
            finishedAt: stateName === "queued" || stateName === "verifying" ? null : AT,
          };
        })
      );
      const artifacts = mode === "succeeded"
        ? attempts.filter((attempt) => attempt.actionKind === "screenshot")
          .map((attempt, index) => ({
            id: uuid(2_000 + state.nextRunNumber + index),
            attemptId: attempt.id,
            relativePath: `fixture/${runId}/${index}.jpg`,
            label: `Fixture screenshot ${index + 1}`,
            kind: "image/jpeg",
            size: 1,
            sha256: "55".repeat(32),
            createdAt: AT,
          }))
        : [];
      const detail = { run, deviceRuns, attempts, artifacts };
      state.runs.unshift(run);
      state.details[runId] = detail;
      persist();
      return clone(detail);
    }

    function cancelRun(runId: string): void {
      const detail = state.details[runId];
      if (!detail) throw { code: "RunNotFound", message: "Flow run was not found." };
      const run = detail.run as JsonRecord;
      run.state = "cancelled";
      run.eventRevision = (run.eventRevision as number) + 1;
      run.updatedAt = AT;
      for (const device of detail.deviceRuns as JsonRecord[]) {
        device.state = "cancelled";
        device.finishedAt = AT;
        device.releaseProof = {
          udid: device.udid,
          owner: "script",
          hadSession: false,
          hadStream: false,
        };
      }
      for (const attempt of detail.attempts as JsonRecord[]) {
        attempt.state = "cancelled";
        attempt.updatedAt = AT;
        attempt.finishedAt = AT;
      }
      state.runs = state.runs.map((candidate) => candidate.id === runId ? run : candidate);
      persist();
    }

    const supportedLegacy = () => ({
      schemaVersion: 2,
      id: uuid(700 + state.nextRunNumber),
      name: "Imported legacy flow",
      revision: 0,
      entryNodeId: uuid(710 + state.nextRunNumber),
      nodes: [
        { id: uuid(710 + state.nextRunNumber), kind: "start", position: { x: 0, y: 80 }, config: {}, postcondition: null },
        { id: uuid(711 + state.nextRunNumber), kind: "wait", position: { x: 240, y: 80 }, config: { durationMs: 250 }, postcondition: null },
        { id: uuid(712 + state.nextRunNumber), kind: "end", position: { x: 480, y: 80 }, config: {}, postcondition: null },
      ],
      edges: [
        { id: uuid(720 + state.nextRunNumber), sourceNodeId: uuid(710 + state.nextRunNumber), sourcePort: "flow", targetNodeId: uuid(711 + state.nextRunNumber), targetPort: "flow" },
        { id: uuid(721 + state.nextRunNumber), sourceNodeId: uuid(711 + state.nextRunNumber), sourcePort: "flow", targetNodeId: uuid(712 + state.nextRunNumber), targetPort: "flow" },
      ],
      viewport: { x: 0, y: 0, zoom: 1 },
    });

    commandHandlers.set("list_devices", () => clone(devices));
    commandHandlers.set("refresh_devices", () => clone(devices));
    commandHandlers.set("list_jobs", () => []);
    commandHandlers.set("get_stream_settings", () => ({
      fps: 24,
      tileSize: "medium",
      gridQuality: "medium",
      focusQuality: "high",
    }));
    commandHandlers.set("auth_session", () => ({ showAuthUi: false, bypassed: true, user: null }));
    commandHandlers.set("flow_action_catalog", () => clone(catalog));
    commandHandlers.set("flow_list", () => {
      const seen = new Set<string>();
      return [...state.revisions].reverse().flatMap((revision) => {
        const id = (revision.document as JsonRecord).id as string;
        if (seen.has(id)) return [];
        seen.add(id);
        return [summaryFor(revision)];
      });
    });
    commandHandlers.set("flow_get", (args) => {
      const revision = [...state.revisions].reverse().find((candidate) => {
        const document = candidate.document as JsonRecord;
        return document.id === args.id &&
          (args.revision === null || args.revision === undefined || document.revision === args.revision);
      });
      return revision ? clone(revision) : null;
    });
    commandHandlers.set("flow_validate", (args) => {
      const document = args.document as JsonRecord;
      const invalidWait = (document.nodes as JsonRecord[]).find((node) =>
        node.kind === "wait" &&
        typeof (node.config as JsonRecord).durationMs === "number" &&
        ((node.config as JsonRecord).durationMs as number) > 60_000);
      if (invalidWait) {
        throw [{
          code: "WaitOutOfRange",
          message: "Wait duration must be between 1 and 60000 ms.",
          nodeId: invalidWait.id,
          field: "config.durationMs",
        }];
      }
      return compiledFor(document);
    });
    commandHandlers.set("flow_save_revision", (args) => {
      const submitted = clone(args.document as JsonRecord);
      const latest = [...state.revisions].reverse().find((candidate) =>
        (candidate.document as JsonRecord).id === submitted.id);
      const latestRevision = latest ? ((latest.document as JsonRecord).revision as number) : 0;
      if (args.expectedRevision !== null && args.expectedRevision !== latestRevision) {
        throw { code: "RevisionConflict", message: "The Flow was changed elsewhere." };
      }
      submitted.revision = latestRevision + 1;
      const record = revisionFor(submitted);
      state.revisions.push(record);
      persist();
      return clone(record);
    });
    commandHandlers.set("flow_archive", (args) => {
      state.revisions = state.revisions.filter((record) => (record.document as JsonRecord).id !== args.id);
      persist();
      return null;
    });
    commandHandlers.set("flow_export", (args) => {
      const revision = [...state.revisions].reverse().find((candidate) =>
        (candidate.document as JsonRecord).id === args.id);
      if (!revision) throw { code: "FlowNotFound", message: "Flow was not found." };
      return JSON.stringify(revision.document, null, 2);
    });
    commandHandlers.set("flow_import_legacy", (args) => {
      let parsed: JsonRecord;
      try {
        parsed = JSON.parse(args.scriptJson as string) as JsonRecord;
      } catch {
        throw { code: "InvalidLegacyJson", message: "Legacy script JSON is invalid." };
      }
      const steps = Array.isArray(parsed.steps) ? parsed.steps as JsonRecord[] : [];
      const invalidIndex = steps.findIndex((step) =>
        step.action === "wait" && typeof step.milliseconds === "number" && step.milliseconds > 60_000);
      if (invalidIndex >= 0) {
        return {
          document: null,
          diagnostics: [{
            stepIndex: invalidIndex,
            code: "WaitOutOfRange",
            message: "Wait duration must be between 1 and 60000 ms.",
            field: "milliseconds",
          }],
        };
      }
      return { document: supportedLegacy(), diagnostics: [] };
    });
    commandHandlers.set("flow_run", (args) => {
      const mode = volatileRunMode;
      volatileRunMode = "succeeded";
      return (createRun(args, mode).run as JsonRecord);
    });
    commandHandlers.set("flow_cancel_run", (args) => {
      cancelRun(args.runId as string);
      return null;
    });
    commandHandlers.set("flow_retry_attempt", (args) => {
      const detail = Object.values(state.details).find((candidate) =>
        (candidate.attempts as JsonRecord[]).some((attempt) => attempt.id === args.attemptId));
      const attempt = (detail?.attempts as JsonRecord[] | undefined)?.find(
        (candidate) => candidate.id === args.attemptId);
      if (!detail || !attempt || attempt.retryAllowed !== true) {
        throw {
          code: attempt?.state === "uncertain" ? "RetryNotAllowed" : "AttemptNotFound",
          message: "This attempt is not safe to retry.",
          attemptId: args.attemptId,
        };
      }
      const retry = { ...attempt, id: uuid(state.nextRunNumber++), attemptNo: (attempt.attemptNo as number) + 1, state: "succeeded", retryAllowed: false };
      (detail.attempts as JsonRecord[]).push(retry);
      persist();
      return clone(retry);
    });
    commandHandlers.set("flow_list_runs", () => clone(state.runs));
    commandHandlers.set("flow_get_run", (args) => clone(state.details[args.runId as string] ?? null));
    commandHandlers.set("flow_coordinate_frame", () => ({
      jpegBase64: JPEG_BASE64,
      imageWidth: 375,
      imageHeight: 667,
      orientation: "portrait",
      profileId: "11".repeat(32),
    }));
    commandHandlers.set("flow_read_artifact", (args) => ({
      artifactId: args.artifactId,
      label: "Fixture screenshot",
      kind: "image/jpeg",
      size: 1,
      sha256: "55".repeat(32),
      base64: JPEG_BASE64,
    }));
    commandHandlers.set("example_script", () => JSON.stringify({
      version: 1,
      name: "fixture",
      steps: [{ action: "wait", milliseconds: 250 }],
    }, null, 2));
    commandHandlers.set("list_scripts", () => [["fixture", JSON.stringify({
      version: 1,
      name: "fixture",
      steps: [{ action: "wait", milliseconds: 250 }],
    })]]);
    commandHandlers.set("save_script", () => null);
    commandHandlers.set("list_schedules", () => []);
    // Both fleet-health probes. `invoke` throws `Unknown mock command` for anything
    // unregistered, so an added probe breaks every spec until it is listed here — which
    // is the point: the registry is the contract, not a convenience.
    commandHandlers.set("driver_degraded_reason", () => null);
    commandHandlers.set("android_unavailable_reason", () => null);
    // Registered because `invoke` throws `Unknown mock command` for anything absent,
    // and the Settings tab calls neither on mount but offers both as buttons.
    commandHandlers.set("update_check", () => ({
      available: false,
      version: null,
      current: "0.1.0",
      busyReason: null,
    }));
    commandHandlers.set("update_install", () => null);

    async function invoke(command: string, args: JsonRecord = {}) {
      if (command === "plugin:event|listen") {
        const handlerId = args.handler;
        if (typeof handlerId !== "number" || !callbacks.has(handlerId)) {
          throw new Error("Invalid mock event handler");
        }
        const listenerId = nextListenerId++;
        listeners.set(listenerId, handlerId);
        return listenerId;
      }
      if (command === "plugin:event|unlisten") {
        const listenerId = args.eventId;
        if (typeof listenerId !== "number") throw new Error("Invalid mock listener ID");
        unregisterListener(listenerId);
        return null;
      }
      commandCalls.push({ command, args: clone(args) });
      const handler = commandHandlers.get(command);
      if (!handler) throw new Error(`Unknown mock command: ${command}`);
      return handler(args);
    }

    const mockWindow = window as typeof window & {
      __TAURI_INTERNALS__?: JsonRecord;
      __TAURI_EVENT_PLUGIN_INTERNALS__?: JsonRecord;
      __RIVIU_TEST__?: JsonRecord;
    };
    mockWindow.__TAURI_INTERNALS__ = {
      ...mockWindow.__TAURI_INTERNALS__,
      transformCallback,
      unregisterCallback,
      invoke,
    };
    mockWindow.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
      unregisterListener: (_event: string, listenerId: number) => {
        unregisterListener(listenerId);
      },
    };
    mockWindow.__RIVIU_TEST__ = {
      emit,
      calls: () => clone(commandCalls),
      setNextRunMode: (mode: MockRunMode) => {
        volatileRunMode = mode;
      },
      state: () => clone(state),
      resetBackend: () => {
        state = initialState();
        persist();
      },
    };
  }, options);
}

export async function setNextRunMode(page: Page, mode: MockRunMode): Promise<void> {
  await page.evaluate((nextMode) => {
    const hook = (window as typeof window & {
      __RIVIU_TEST__: { setNextRunMode: (value: MockRunMode) => void };
    }).__RIVIU_TEST__;
    hook.setNextRunMode(nextMode);
  }, mode);
}

export async function emitRiviuEvent(page: Page, payload: unknown): Promise<void> {
  await page.evaluate((eventPayload) => {
    const hook = (window as typeof window & {
      __RIVIU_TEST__: { emit: (value: unknown) => void };
    }).__RIVIU_TEST__;
    hook.emit(eventPayload);
  }, payload);
}

export async function mockCommandCalls(page: Page): Promise<MockCommandCall[]> {
  return page.evaluate(() => {
    const hook = (window as typeof window & {
      __RIVIU_TEST__: { calls: () => MockCommandCall[] };
    }).__RIVIU_TEST__;
    return hook.calls();
  });
}
