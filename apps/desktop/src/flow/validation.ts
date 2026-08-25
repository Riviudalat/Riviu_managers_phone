import type {
  ActionKind,
  CommandError,
  FlowDocumentV2,
  FlowValidationIssue,
  JsonObject,
  JsonValue,
} from "../types";
import { describeError } from "../describeError";

const ACTION_KINDS = new Set<ActionKind>([
  "start",
  "end",
  "launchApp",
  "terminateApp",
  "wait",
  "tap",
  "swipe",
  "typeText",
  "screenshot",
  "home",
  "assertVisible",
  "rawHttp",
  "rawWda",
  "shell",
]);

export interface NumberInputOptions {
  integer?: boolean;
  minimum?: number;
  maximum?: number;
}

export function parseFiniteNumberInput(
  rawValue: string,
  options: NumberInputOptions = {},
): number | null {
  if (rawValue.trim() === "") return null;
  const value = Number(rawValue);
  if (!Number.isFinite(value)) return null;
  if (options.integer && !Number.isInteger(value)) return null;
  if (options.minimum !== undefined && value < options.minimum) return null;
  if (options.maximum !== undefined && value > options.maximum) return null;
  return value;
}

export function acceptFiniteValueAsNumber(
  rawValue: string,
  valueAsNumber: number,
  options: NumberInputOptions = {},
): number | null {
  if (rawValue.trim() === "" || !Number.isFinite(valueAsNumber)) return null;
  if (options.integer && !Number.isInteger(valueAsNumber)) return null;
  if (options.minimum !== undefined && valueAsNumber < options.minimum) return null;
  if (options.maximum !== undefined && valueAsNumber > options.maximum) return null;
  return valueAsNumber;
}

export function validateDraftNumbers(document: FlowDocumentV2): FlowValidationIssue[] {
  const issues: FlowValidationIssue[] = [];
  for (const [field, value] of [
    ["viewport.x", document.viewport.x],
    ["viewport.y", document.viewport.y],
    ["viewport.zoom", document.viewport.zoom],
  ] as const) {
    if (!Number.isFinite(value)) issues.push(numberIssue(field));
  }
  for (const node of document.nodes) {
    if (!Number.isFinite(node.position.x)) {
      issues.push(numberIssue("position.x", node.id));
    }
    if (!Number.isFinite(node.position.y)) {
      issues.push(numberIssue("position.y", node.id));
    }
    if (!isFiniteJsonObject(node.config)) {
      issues.push(numberIssue("config", node.id));
    }
  }
  return issues;
}

export function normalizeFlowIssues(error: unknown): FlowValidationIssue[] {
  if (Array.isArray(error)) {
    const issues = error.filter(isCommandError).map((issue) => ({ ...issue }));
    if (issues.length > 0) return issues;
  }
  if (isCommandError(error)) return [{ ...error }];
  return [{ code: "ValidationTransportFailed", message: describeError(error) }];
}

export function isFlowDocumentV2(value: unknown): value is FlowDocumentV2 {
  if (!isRecord(value)) return false;
  if (
    value.schemaVersion !== 2 ||
    typeof value.id !== "string" ||
    typeof value.name !== "string" ||
    !isNonnegativeInteger(value.revision) ||
    typeof value.entryNodeId !== "string" ||
    !Array.isArray(value.nodes) ||
    !Array.isArray(value.edges) ||
    !isRecord(value.viewport)
  ) {
    return false;
  }
  if (
    !isFiniteNumber(value.viewport.x) ||
    !isFiniteNumber(value.viewport.y) ||
    !isFiniteNumber(value.viewport.zoom)
  ) {
    return false;
  }
  return value.nodes.every(isFlowNode) && value.edges.every(isFlowEdge);
}

function isFlowNode(value: unknown): boolean {
  if (!isRecord(value) || !isRecord(value.position)) return false;
  return (
    typeof value.id === "string" &&
    typeof value.kind === "string" &&
    ACTION_KINDS.has(value.kind as ActionKind) &&
    isFiniteNumber(value.position.x) &&
    isFiniteNumber(value.position.y) &&
    isFiniteJsonObject(value.config) &&
    (value.postcondition === undefined || value.postcondition === null ||
      isFiniteJsonObject(value.postcondition))
  );
}

function isFlowEdge(value: unknown): boolean {
  return (
    isRecord(value) &&
    typeof value.id === "string" &&
    typeof value.sourceNodeId === "string" &&
    typeof value.sourcePort === "string" &&
    typeof value.targetNodeId === "string" &&
    typeof value.targetPort === "string"
  );
}

export function isFiniteJsonObject(value: unknown): value is JsonObject {
  return isRecord(value) && Object.values(value).every(isFiniteJsonValue);
}

export function isFiniteJsonValue(value: unknown): value is JsonValue {
  if (value === null || typeof value === "string" || typeof value === "boolean") return true;
  if (typeof value === "number") return Number.isFinite(value);
  if (Array.isArray(value)) return value.every(isFiniteJsonValue);
  return isFiniteJsonObject(value);
}

function isCommandError(value: unknown): value is CommandError {
  if (!isRecord(value) || typeof value.code !== "string" || typeof value.message !== "string") {
    return false;
  }
  return optionalString(value.nodeId) && optionalString(value.field) &&
    optionalString(value.udid) && optionalString(value.attemptId);
}

function optionalString(value: unknown): boolean {
  return value === undefined || typeof value === "string";
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isFiniteNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function isNonnegativeInteger(value: unknown): value is number {
  return isFiniteNumber(value) && Number.isInteger(value) && value >= 0;
}

function numberIssue(field: string, nodeId?: string): FlowValidationIssue {
  return {
    code: "NonFiniteNumber",
    message: "Enter a finite number.",
    field,
    ...(nodeId === undefined ? {} : { nodeId }),
  };
}

