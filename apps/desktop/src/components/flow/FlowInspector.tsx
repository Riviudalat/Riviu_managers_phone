import { useState } from "react";
import type {
  ActionDefinition,
  EvidenceKind,
  EvidenceSpec,
  FlowCoordinateFrame,
  FlowNode,
  FlowValidationIssue,
  ImageCoordinateTarget,
  JsonObject,
  JsonValue,
  QualifiedElementLocator,
  ScreenOrientation,
} from "../../types";
import { flowCoordinateFrame } from "../../api";
import { acceptFiniteValueAsNumber } from "../../flow/validation";
import { FlowCoordinatePicker } from "./FlowCoordinatePicker";
import { FlowVisionCapture } from "./FlowVisionCapture";

interface JsonSchema {
  type: "object" | "string" | "number" | "integer" | "boolean";
  title?: string;
  minimum?: number;
  maximum?: number;
  minLength?: number;
  maxLength?: number;
  enum?: string[];
  properties?: Record<string, JsonSchema>;
  required?: string[];
}

type CoordinateFieldName = "point" | "from" | "to";

function displayCommandError(error: unknown): string {
  if (typeof error === "object" && error !== null) {
    const value = error as Record<string, unknown>;
    if (typeof value.code === "string" && typeof value.message === "string") {
      return `${value.code}: ${value.message}`;
    }
    if (typeof value.message === "string") return value.message;
  }
  return error instanceof Error ? error.message : String(error);
}

export interface FlowInspectorProps {
  node: FlowNode | null;
  definition: ActionDefinition | null;
  issues: FlowValidationIssue[];
  onConfigChange: (config: JsonObject) => void;
  onPostconditionChange: (postcondition: EvidenceSpec | null) => void;
  coordinateDeviceUdid?: string | null;
  launchBundleId?: string | null;
  loadCoordinateFrame?: () => Promise<FlowCoordinateFrame>;
}

interface SchemaFieldProps {
  name: string;
  schema: JsonSchema;
  value: JsonValue | undefined;
  onChange: (value: JsonValue) => void;
  issues: FlowValidationIssue[];
}

const ORIENTATIONS: ScreenOrientation[] = [
  "portrait",
  "portraitUpsideDown",
  "landscapeLeft",
  "landscapeRight",
];

const EVIDENCE_LABELS: Record<EvidenceKind, string> = {
  activeAppEquals: "Active app equals",
  processAbsent: "Process absent",
  frameDigestChanged: "Frame digest changed",
  frameRegionChanged: "Frame region changed",
  qualifiedFramePredicate: "Qualified frame predicate",
  accessibilityVisible: "Accessibility visible",
  textReadBackEquals: "Text read-back equals",
  artifactDecodedAndHashed: "Artifact decoded and hashed",
};

function isJsonObject(value: JsonValue | undefined): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function titleFor(name: string): string {
  const known: Record<string, string> = {
    accessibilityId: "Accessibility ID",
    bundleId: "Bundle ID",
    detectorId: "Detector ID",
    durationMs: "Duration (ms)",
    format: "Format",
    from: "From",
    imageHeight: "Image height",
    imageWidth: "Image width",
    label: "Label",
    minimumDistance: "Minimum distance",
    point: "Point",
    profileId: "Profile ID",
    readBackLocator: "Read-back locator",
    strategy: "Strategy",
    text: "Text",
    to: "To",
    value: "Locator value",
  };
  return known[name] ?? name.replace(/([a-z])([A-Z])/g, "$1 $2");
}

function issuesForField(
  issues: FlowValidationIssue[],
  field: string,
): FlowValidationIssue[] {
  return issues.filter((issue) => {
    const path = issue.field;
    return path === field || path === `config.${field}` || path?.startsWith(`${field}.`) ||
      path?.startsWith(`config.${field}.`);
  });
}

function FieldIssues({ issues }: { issues: FlowValidationIssue[] }) {
  if (issues.length === 0) return null;
  return (
    <ul className="flow-field-errors">
      {issues.map((issue, index) => (
        <li role="alert" key={`${issue.code}-${index}`}>
          {issue.message}
        </li>
      ))}
    </ul>
  );
}

function SchemaObjectFields({
  schema,
  value,
  onChange,
  issues,
}: {
  schema: JsonSchema;
  value: JsonObject;
  onChange: (value: JsonObject) => void;
  issues: FlowValidationIssue[];
}) {
  return Object.entries(schema.properties ?? {}).map(([name, child]) => (
    <SchemaField
      key={name}
      name={name}
      schema={child}
      value={value[name]}
      issues={issuesForField(issues, name)}
      onChange={(next) => onChange({ ...value, [name]: next })}
    />
  ));
}

function SchemaField({ name, schema, value, onChange, issues }: SchemaFieldProps) {
  const label = schema.title ?? titleFor(name);
  if (schema.enum) {
    return (
      <div className="flow-field-with-errors">
        <label className="flow-field">
          <span>{label}</span>
          <select
            value={typeof value === "string" ? value : ""}
            onChange={(event) => onChange(event.currentTarget.value)}
          >
            {schema.enum.map((option) => (
              <option key={option} value={option}>
                {option}
              </option>
            ))}
          </select>
        </label>
        <FieldIssues issues={issues} />
      </div>
    );
  }

  switch (schema.type) {
    case "string":
      return (
        <div className="flow-field-with-errors">
          <label className="flow-field">
            <span>{label}</span>
            <input
              type="text"
              value={typeof value === "string" ? value : ""}
              minLength={schema.minLength}
              maxLength={schema.maxLength}
              onChange={(event) => {
                const next = event.currentTarget.value;
                if (schema.maxLength === undefined || next.length <= schema.maxLength) {
                  onChange(next);
                }
              }}
            />
          </label>
          <FieldIssues issues={issues} />
        </div>
      );
    case "number":
    case "integer":
      return (
        <div className="flow-field-with-errors">
          <label className="flow-field">
            <span>{label}</span>
            <input
              type="number"
              step={schema.type === "integer" ? 1 : "any"}
              min={schema.minimum}
              max={schema.maximum}
              value={typeof value === "number" && Number.isFinite(value) ? value : ""}
              onChange={(event) => {
                const next = acceptFiniteValueAsNumber(
                  event.currentTarget.value,
                  event.currentTarget.valueAsNumber,
                  {
                    integer: schema.type === "integer",
                    minimum: schema.minimum,
                    maximum: schema.maximum,
                  },
                );
                if (next !== null) onChange(next);
              }}
            />
          </label>
          <FieldIssues issues={issues} />
        </div>
      );
    case "boolean":
      return (
        <div className="flow-field-with-errors">
          <label className="flow-field flow-field-checkbox">
            <input
              type="checkbox"
              checked={value === true}
              onChange={(event) => onChange(event.currentTarget.checked)}
            />
            <span>{label}</span>
          </label>
          <FieldIssues issues={issues} />
        </div>
      );
    case "object":
      return (
        <fieldset className="flow-field-group">
          <legend>{label}</legend>
          <SchemaObjectFields
            schema={schema}
            value={isJsonObject(value) ? value : {}}
            issues={issues}
            onChange={onChange}
          />
          <FieldIssues issues={issues.filter((issue) => issue.field === name)} />
        </fieldset>
      );
    default:
      throw new Error("UnsupportedFieldSchema");
  }
}

function locatorFromValue(value: JsonValue | undefined): QualifiedElementLocator {
  if (isJsonObject(value)) {
    const strategy = value.strategy;
    const locatorValue = value.value;
    if (
      (strategy === "accessibilityId" || strategy === "className") &&
      typeof locatorValue === "string"
    ) {
      return { strategy, value: locatorValue };
    }
  }
  return { strategy: "accessibilityId", value: "" };
}

function ReadBackLocatorFields({
  value,
  onChange,
  issues,
}: {
  value: JsonValue | QualifiedElementLocator | undefined;
  onChange: (value: QualifiedElementLocator) => void;
  issues: FlowValidationIssue[];
}) {
  const locator = locatorFromValue(value as JsonValue | undefined);
  return (
    <fieldset className="flow-field-group">
      <legend>Read-back locator</legend>
      <div role="group" aria-label="Locator strategy" className="flow-segmented-control">
        {(["accessibilityId", "className"] as const).map((strategy) => (
          <button
            type="button"
            key={strategy}
            aria-pressed={locator.strategy === strategy}
            onClick={() => onChange({ ...locator, strategy })}
          >
            {strategy === "accessibilityId" ? "Accessibility ID" : "Class name"}
          </button>
        ))}
      </div>
      <label className="flow-field">
        <span>Locator value</span>
        <input
          type="text"
          value={locator.value}
          maxLength={512}
          onChange={(event) => onChange({ ...locator, value: event.currentTarget.value })}
        />
      </label>
      <FieldIssues issues={issues} />
    </fieldset>
  );
}

function coordinateFromValue(value: JsonValue | undefined): ImageCoordinateTarget {
  const object = isJsonObject(value) ? value : {};
  const finite = (key: string, fallback: number) =>
    typeof object[key] === "number" && Number.isFinite(object[key])
      ? (object[key] as number)
      : fallback;
  const orientation = ORIENTATIONS.includes(object.orientation as ScreenOrientation)
    ? (object.orientation as ScreenOrientation)
    : "portrait";
  return {
    x: finite("x", 0),
    y: finite("y", 0),
    imageWidth: finite("imageWidth", 1),
    imageHeight: finite("imageHeight", 1),
    orientation,
    profileId: typeof object.profileId === "string" ? object.profileId : "",
  };
}

function coordinateToJson(value: ImageCoordinateTarget): JsonObject {
  return {
    x: value.x,
    y: value.y,
    imageWidth: value.imageWidth,
    imageHeight: value.imageHeight,
    orientation: value.orientation,
    profileId: value.profileId,
  };
}

function CoordinateFields({
  label,
  field,
  value,
  issues,
  onChange,
  onPick,
  canPick,
}: {
  label: string;
  field: CoordinateFieldName;
  value: JsonValue | undefined;
  issues: FlowValidationIssue[];
  onChange: (value: ImageCoordinateTarget) => void;
  onPick: (field: CoordinateFieldName) => void;
  canPick: boolean;
}) {
  const coordinate = coordinateFromValue(value);
  const numericField = (
    name: "x" | "y" | "imageWidth" | "imageHeight",
    title: string,
    integer: boolean,
    minimum?: number,
  ) => (
    <label className="flow-field">
      <span>{title}</span>
      <input
        type="number"
        step={integer ? 1 : "any"}
        min={minimum}
        value={coordinate[name]}
        onChange={(event) => {
          const next = acceptFiniteValueAsNumber(
            event.currentTarget.value,
            event.currentTarget.valueAsNumber,
            { integer, minimum },
          );
          if (next !== null) onChange({ ...coordinate, [name]: next });
        }}
      />
    </label>
  );

  return (
    <fieldset className="flow-coordinate-fields">
      <legend>{label}</legend>
      <div className="flow-coordinate-pair">
        {numericField("x", "X", false)}
        {numericField("y", "Y", false)}
      </div>
      <div className="flow-coordinate-pair">
        {numericField("imageWidth", "Image width", true, 1)}
        {numericField("imageHeight", "Image height", true, 1)}
      </div>
      <label className="flow-field">
        <span>Orientation</span>
        <select
          value={coordinate.orientation}
          onChange={(event) =>
            onChange({ ...coordinate, orientation: event.currentTarget.value as ScreenOrientation })
          }
        >
          {ORIENTATIONS.map((orientation) => (
            <option key={orientation} value={orientation}>
              {orientation}
            </option>
          ))}
        </select>
      </label>
      <label className="flow-field">
        <span>Profile ID</span>
        <input type="text" value={coordinate.profileId} readOnly />
      </label>
      <button type="button" disabled={!canPick} onClick={() => onPick(field)}>
        Pick {label.toLowerCase()} from device
      </button>
      <FieldIssues issues={issues} />
    </fieldset>
  );
}

function availableEvidence(definition: ActionDefinition): EvidenceKind[] {
  return definition.allowedEvidence.filter(
    (kind) => kind !== "qualifiedFramePredicate" || definition.qualifiedDetectorIds.length > 0,
  );
}

function defaultEvidence(
  kind: EvidenceKind,
  node: FlowNode,
  definition: ActionDefinition,
  launchBundleId: string | null,
): EvidenceSpec {
  const stringConfig = (name: string) =>
    typeof node.config[name] === "string" ? (node.config[name] as string) : "";
  switch (kind) {
    case "activeAppEquals":
      return { kind, bundleId: stringConfig("bundleId") || launchBundleId || "" };
    case "processAbsent":
      return { kind, bundleId: stringConfig("bundleId") };
    case "frameDigestChanged":
      return { kind, minimumDistance: 1 };
    case "frameRegionChanged":
      return { kind, x: 0, y: 0, width: 1, height: 1, minimumDistance: 1 };
    case "qualifiedFramePredicate":
      return { kind, detectorId: definition.qualifiedDetectorIds[0] ?? "" };
    case "accessibilityVisible":
      return { kind, accessibilityId: stringConfig("accessibilityId") };
    case "textReadBackEquals":
      return {
        kind,
        locator: locatorFromValue(node.config.readBackLocator),
        value: stringConfig("text"),
      };
    case "artifactDecodedAndHashed":
      return { kind };
  }
}

function EvidenceFields({
  node,
  definition,
  issues,
  launchBundleId,
  onChange,
}: {
  node: FlowNode;
  definition: ActionDefinition;
  issues: FlowValidationIssue[];
  launchBundleId: string | null;
  onChange: (value: EvidenceSpec | null) => void;
}) {
  const allowed = availableEvidence(definition);
  if (allowed.length === 0) return <p className="flow-inspector-empty">No evidence fields</p>;
  const currentKind = node.postcondition?.kind ?? "";
  const postcondition =
    node.postcondition !== null && node.postcondition !== undefined &&
      allowed.includes(node.postcondition.kind)
      ? node.postcondition
      : null;

  return (
    <fieldset className="flow-field-group">
      <legend>Evidence</legend>
      <label className="flow-field">
        <span>Evidence type</span>
        <select
          aria-label="Evidence type"
          value={allowed.includes(currentKind as EvidenceKind) ? currentKind : ""}
          onChange={(event) => {
            const kind = event.currentTarget.value as EvidenceKind;
            onChange(kind ? defaultEvidence(kind, node, definition, launchBundleId) : null);
          }}
        >
          <option value="">Select evidence</option>
          {allowed.map((kind) => (
            <option key={kind} value={kind}>
              {EVIDENCE_LABELS[kind]}
            </option>
          ))}
        </select>
      </label>
      {postcondition?.kind === "activeAppEquals" && (
        <label className="flow-field">
          <span>Expected bundle ID</span>
          <input
            type="text"
            readOnly={typeof node.config.bundleId === "string"}
            value={postcondition.bundleId}
            onChange={(event) =>
              onChange({ kind: "activeAppEquals", bundleId: event.currentTarget.value })
            }
          />
        </label>
      )}
      {postcondition?.kind === "processAbsent" && (
        <label className="flow-field">
          <span>Expected absent bundle ID</span>
          <input type="text" readOnly value={postcondition.bundleId} />
        </label>
      )}
      {postcondition?.kind === "frameDigestChanged" && (
        <FiniteEvidenceInput
          label="Minimum distance"
          value={postcondition.minimumDistance}
          onChange={(minimumDistance) => onChange({ kind: "frameDigestChanged", minimumDistance })}
        />
      )}
      {postcondition?.kind === "frameRegionChanged" && (
        <div className="flow-evidence-grid">
          {(["x", "y", "width", "height", "minimumDistance"] as const).map((field) => (
            <FiniteEvidenceInput
              key={field}
              label={titleFor(field)}
              value={postcondition[field]}
              minimum={field === "width" || field === "height" ? 1 : undefined}
              onChange={(value) => {
                onChange({ ...postcondition, [field]: value });
              }}
            />
          ))}
        </div>
      )}
      {postcondition?.kind === "qualifiedFramePredicate" && (
        <label className="flow-field">
          <span>Detector ID</span>
          <select
            value={postcondition.detectorId}
            onChange={(event) =>
              onChange({ kind: "qualifiedFramePredicate", detectorId: event.currentTarget.value })
            }
          >
            {definition.qualifiedDetectorIds.map((id) => (
              <option key={id} value={id}>{id}</option>
            ))}
          </select>
        </label>
      )}
      {postcondition?.kind === "accessibilityVisible" && (
        <label className="flow-field">
          <span>Accessibility ID</span>
          <input
            type="text"
            value={postcondition.accessibilityId}
            onChange={(event) =>
              onChange({ kind: "accessibilityVisible", accessibilityId: event.currentTarget.value })
            }
          />
        </label>
      )}
      {postcondition?.kind === "textReadBackEquals" && (
        <>
          <ReadBackLocatorFields
            value={postcondition.locator}
            issues={issuesForField(issues, "postcondition.locator")}
            onChange={(locator) => onChange({ ...postcondition, locator })}
          />
          <label className="flow-field">
            <span>Expected text</span>
            <input type="text" readOnly value={postcondition.value} />
          </label>
        </>
      )}
      {postcondition?.kind === "artifactDecodedAndHashed" && (
        <p>Decoded image and SHA-256 required</p>
      )}
      <FieldIssues issues={issues.filter((issue) => issue.field?.startsWith("postcondition"))} />
    </fieldset>
  );
}

function FiniteEvidenceInput({
  label,
  value,
  minimum,
  onChange,
}: {
  label: string;
  value: number;
  minimum?: number;
  onChange: (value: number) => void;
}) {
  return (
    <label className="flow-field">
      <span>{label}</span>
      <input
        type="number"
        step="any"
        min={minimum}
        value={Number.isFinite(value) ? value : ""}
        onChange={(event) => {
          const next = acceptFiniteValueAsNumber(
            event.currentTarget.value,
            event.currentTarget.valueAsNumber,
            { minimum },
          );
          if (next !== null) onChange(next);
        }}
      />
    </label>
  );
}

export function FlowInspector({
  node,
  definition,
  issues,
  onConfigChange,
  onPostconditionChange,
  coordinateDeviceUdid = null,
  launchBundleId = null,
  loadCoordinateFrame,
}: FlowInspectorProps) {
  const [picker, setPicker] = useState<{
    field: CoordinateFieldName;
    frame: FlowCoordinateFrame;
  } | null>(null);
  const [pickerError, setPickerError] = useState<string | null>(null);
  const [pickerLoading, setPickerLoading] = useState(false);
  const [visionFrame, setVisionFrame] = useState<FlowCoordinateFrame | null>(null);

  if (node === null || definition === null) {
    return (
      <aside className="flow-inspector" aria-label="Flow inspector" data-testid="flow-inspector">
        <p>Select an action to edit it.</p>
      </aside>
    );
  }

  const nodeIssues = issues.filter((issue) => issue.nodeId === node.id);
  const schema = definition.configSchema as JsonSchema | null;
  const coordinateAvailable =
    loadCoordinateFrame !== undefined ||
    (coordinateDeviceUdid !== null && launchBundleId !== null);

  const commitConfig = (config: JsonObject) => {
    onConfigChange(config);
    const bundleId = typeof config.bundleId === "string" ? config.bundleId : "";
    if (node.postcondition?.kind === "processAbsent") {
      onPostconditionChange({ kind: "processAbsent", bundleId });
    } else if (node.postcondition?.kind === "activeAppEquals") {
      onPostconditionChange({ kind: "activeAppEquals", bundleId });
    } else if (node.postcondition?.kind === "textReadBackEquals") {
      const locator = locatorFromValue(config.readBackLocator);
      const value = typeof config.text === "string" ? config.text : "";
      onPostconditionChange({ kind: "textReadBackEquals", locator, value });
    }
  };

  const updateConfigField = (name: string, value: JsonValue) => {
    commitConfig({ ...node.config, [name]: value });
  };

  const requestCoordinate = async (field: CoordinateFieldName) => {
    setPickerError(null);
    setPickerLoading(true);
    try {
      const frame = loadCoordinateFrame
        ? await loadCoordinateFrame()
        : await flowCoordinateFrame(coordinateDeviceUdid ?? "", launchBundleId ?? "");
      setPicker({ field, frame });
    } catch (error) {
      setPickerError(displayCommandError(error));
    } finally {
      setPickerLoading(false);
    }
  };

  const requestVisionFrame = async () => {
    setPickerError(null);
    setPickerLoading(true);
    try {
      const frame = loadCoordinateFrame
        ? await loadCoordinateFrame()
        : await flowCoordinateFrame(coordinateDeviceUdid ?? "", launchBundleId ?? "");
      setVisionFrame(frame);
    } catch (error) {
      setPickerError(displayCommandError(error));
    } finally {
      setPickerLoading(false);
    }
  };

  const handleTemplateUpload = async (event: React.ChangeEvent<HTMLInputElement>) => {
    const file = event.currentTarget.files?.[0];
    event.currentTarget.value = "";
    if (!file) return;
    try {
      const bytes = new Uint8Array(await file.arrayBuffer());
      let binary = "";
      for (const byte of bytes) binary += String.fromCharCode(byte);
      const next: JsonObject = { ...node.config, templatePngBase64: btoa(binary) };
      delete next.region;
      commitConfig(next);
    } catch (error) {
      setPickerError(displayCommandError(error));
    }
  };

  const renderCoordinate = (
    field: CoordinateFieldName,
    value: JsonValue | undefined,
    onChange: (value: ImageCoordinateTarget) => void,
  ) => (
    <CoordinateFields
      key={field}
      label={titleFor(field)}
      field={field}
      value={value}
      issues={issuesForField(nodeIssues, field)}
      onChange={onChange}
      onPick={(nextField) => void requestCoordinate(nextField)}
      canPick={coordinateAvailable && !pickerLoading}
    />
  );

  const renderConfigFields = () => {
    if (schema === null) return <p>This action has no editable schema.</p>;
    if (schema.type !== "object") throw new Error("UnsupportedFieldSchema");

    if (node.kind === "tapVision" || node.kind === "ifVision") {
      const template =
        typeof node.config.templatePngBase64 === "string" ? node.config.templatePngBase64 : "";
      return (
        <>
          <div className="flow-field">
            <span>Ảnh mẫu (template)</span>
            {template ? (
              <img
                className="flow-vision-template-preview"
                src={`data:image/png;base64,${template}`}
                alt="Template preview"
              />
            ) : (
              <p className="flow-inspector-empty">Chưa có mẫu — chụp từ thiết bị hoặc tải PNG.</p>
            )}
            <div className="flow-vision-template-actions">
              <button
                type="button"
                disabled={!coordinateAvailable || pickerLoading}
                onClick={() => void requestVisionFrame()}
              >
                Chụp mẫu từ thiết bị
              </button>
              <label className="flow-vision-upload">
                Tải PNG
                <input type="file" accept="image/png" onChange={(event) => void handleTemplateUpload(event)} />
              </label>
            </div>
          </div>
          <SchemaField
            name="threshold"
            schema={
              schema.properties?.threshold ?? { type: "number", minimum: 0, maximum: 1 }
            }
            value={node.config.threshold}
            issues={issuesForField(nodeIssues, "threshold")}
            onChange={(value) => updateConfigField("threshold", value)}
          />
          {isJsonObject(node.config.region) && (
            <button
              type="button"
              onClick={() => {
                const next: JsonObject = { ...node.config };
                delete next.region;
                commitConfig(next);
              }}
            >
              Xóa vùng tìm (tìm toàn màn hình)
            </button>
          )}
        </>
      );
    }

    if (node.kind === "tap") {
      const pointMode = isJsonObject(node.config.point);
      return (
        <>
          <div role="group" aria-label="Tap target mode" className="flow-segmented-control">
            <button
              type="button"
              aria-pressed={!pointMode}
              onClick={() => commitConfig({ accessibilityId: "" })}
            >
              Accessibility ID
            </button>
            <button
              type="button"
              aria-pressed={pointMode}
              onClick={() =>
                commitConfig({ point: coordinateToJson(coordinateFromValue(node.config.point)) })
              }
            >
              Coordinate
            </button>
          </div>
          {pointMode
            ? renderCoordinate("point", node.config.point, (value) =>
                commitConfig({ point: coordinateToJson(value) }))
            : (
                <SchemaField
                  name="accessibilityId"
                  schema={schema.properties?.accessibilityId ?? { type: "string" }}
                  value={node.config.accessibilityId}
                  issues={issuesForField(nodeIssues, "accessibilityId")}
                  onChange={(value) => commitConfig({ accessibilityId: value })}
                />
              )}
        </>
      );
    }

    return Object.entries(schema.properties ?? {}).map(([name, child]) => {
      if (name === "point" || name === "from" || name === "to") {
        return renderCoordinate(name, node.config[name], (value) =>
          updateConfigField(name, coordinateToJson(value)));
      }
      if (name === "readBackLocator") {
        return (
          <ReadBackLocatorFields
            key={name}
            value={node.config[name]}
            issues={issuesForField(nodeIssues, name)}
            onChange={(value) => updateConfigField(name, value)}
          />
        );
      }
      return (
        <SchemaField
          key={name}
          name={name}
          schema={child}
          value={node.config[name]}
          issues={issuesForField(nodeIssues, name)}
          onChange={(value) => updateConfigField(name, value)}
        />
      );
    });
  };

  return (
    <aside className="flow-inspector" aria-label="Flow inspector" data-testid="flow-inspector">
      <header>
        <strong>{definition.label}</strong>
        <span>{node.id}</span>
      </header>
      {definition.disabledReason && <p role="alert">{definition.disabledReason}</p>}
      <FieldIssues issues={nodeIssues.filter((issue) => !issue.field)} />
      <section aria-label="Action configuration">{renderConfigFields()}</section>
      <EvidenceFields
        node={node}
        definition={definition}
        issues={nodeIssues}
        launchBundleId={launchBundleId}
        onChange={onPostconditionChange}
      />
      {pickerLoading && <p role="status">Loading device frame...</p>}
      {pickerError && <p role="alert">{pickerError}</p>}
      {visionFrame && (
        <section className="flow-coordinate-popover" aria-label="Capture template">
          <FlowVisionCapture
            frame={visionFrame}
            onCapture={(templatePngBase64, region) => {
              commitConfig({
                ...node.config,
                templatePngBase64,
                region: { x0: region.x0, y0: region.y0, x1: region.x1, y1: region.y1 },
              });
              setVisionFrame(null);
            }}
            onCancel={() => setVisionFrame(null)}
          />
        </section>
      )}
      {picker && (
        <section className="flow-coordinate-popover" aria-label={`Pick ${picker.field}`}>
          <FlowCoordinatePicker
            frame={picker.frame}
            onPick={(value) => {
              if (node.kind === "tap" && picker.field === "point") {
                commitConfig({ point: coordinateToJson(value) });
              } else {
                updateConfigField(picker.field, coordinateToJson(value));
              }
              setPicker(null);
            }}
          />
          <button type="button" onClick={() => setPicker(null)}>Cancel picker</button>
        </section>
      )}
    </aside>
  );
}
