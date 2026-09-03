import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  ActionDefinition,
  ActionKind,
  DeviceInfo,
  EvidenceSpec,
  FlowCoordinateFrame,
  FlowNode,
  JsonObject,
  JsonValue,
} from "../../types";
import { summarizeAction } from "./actionPresentation";
import { FlowDiagnostics } from "./FlowDiagnostics";
import { FlowInspector } from "./FlowInspector";
import { FlowRunDialog } from "./FlowRunDialog";
import { FlowVisionCapture } from "./FlowVisionCapture";

/**
 * The vision-capture path and its neighbours, all of which had the same shape of defect: a result
 * that arrives later than the thing it was about.
 *
 * A device frame is fetched asynchronously and cropped asynchronously, while the node it belongs to
 * is a *prop* that the canvas can change underneath at any moment. None of the transient state was
 * keyed to a node, so a capture started on one node could be written into another — under the other
 * node's name, with nothing on screen to say it had happened.
 */

afterEach(cleanup);

const VISION_SCHEMA: JsonValue = {
  type: "object",
  properties: {
    templatePngBase64: { type: "string" },
    threshold: { type: "number", minimum: 0, maximum: 1 },
    region: { type: "object" },
  },
};

// A 1x1 JPEG is enough: these cases are about *when* the crop applies, not about pixels.
const FRAME: FlowCoordinateFrame = {
  jpegBase64:
    "/9j/4AAQSkZJRgABAQEAYABgAAD/2wBDAAgGBgcGBQgHBwcJCQgKDBQNDAsLDBkSEw8UHRofHh0aHBwc" +
    "JC4nICIsIxwcKDcpLDAxNDQ0Hyc5PTgyPDIzNP/AABEIAAEAAQMBIgACEQEDEQH/xAAfAAABBQEBAQEB" +
    "AQAAAAAAAAAAAQIDBAUGBwgJCgv/xAC1EAACAQMDAgQDBQUEBAAAAX0BAgMABBEFEiExQQYTUWEHInEU" +
    "MoGRoQgjQlKx/9oACAEBAAA/AH8A/9k=",
  imageWidth: 400,
  imageHeight: 800,
  orientation: "portrait",
  profileId: "fixture",
};

function definition(kind: ActionKind): ActionDefinition {
  return {
    kind,
    schemaVersion: 1,
    label: kind,
    disabledReason: null,
    category: "input",
    configSchema: VISION_SCHEMA,
    inputPorts: [],
    outputPorts: [],
    requiredCapabilities: [],
    resourceClass: "uiWithStream",
    sideEffectClass: "ambiguousUi",
    evidenceRequirement: "none",
    allowedEvidence: [],
    qualifiedDetectorIds: [],
    reconciliationPolicy: "none",
    defaultTimeoutMs: 5_000,
    retryPolicy: "beforeDispatchOnly",
  };
}

function visionNode(id: string): FlowNode {
  return {
    id,
    kind: "tapVision",
    position: { x: 0, y: 0 },
    config: { threshold: 0.9 },
    postcondition: null,
  };
}

describe("a template capture belongs to the node it was started on", () => {
  /** Two vision nodes and a button to switch between them, the way the canvas does. */
  function SwitchingHarness({
    loadCoordinateFrame,
    onConfigFor,
  }: {
    loadCoordinateFrame: () => Promise<FlowCoordinateFrame>;
    onConfigFor: (nodeId: string, config: JsonObject) => void;
  }) {
    const [nodes, setNodes] = useState([visionNode("node-a"), visionNode("node-b")]);
    const [selected, setSelected] = useState("node-a");
    const current = nodes.find((node) => node.id === selected) ?? nodes[0];
    return (
      <>
        <button type="button" onClick={() => setSelected("node-b")}>
          chọn node B
        </button>
        <FlowInspector
          node={current}
          definition={definition("tapVision")}
          issues={[]}
          loadCoordinateFrame={loadCoordinateFrame}
          onConfigChange={(config) => {
            onConfigFor(current.id, config);
            setNodes((all) =>
              all.map((node) => (node.id === current.id ? { ...node, config } : node)),
            );
          }}
          onPostconditionChange={() => undefined}
        />
      </>
    );
  }

  it("closes the capture when the selection moves, instead of re-pointing it", async () => {
    const onConfigFor = vi.fn();
    render(
      <SwitchingHarness
        loadCoordinateFrame={async () => FRAME}
        onConfigFor={onConfigFor}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Chụp mẫu từ thiết bị" }));
    // The capture opened for node A.
    expect(await screen.findByRole("img", { name: "Khung hình thiết bị" })).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "chọn node B" }));

    // Node B is not offered a capture that was never started on it. Before this, the popover
    // stayed open and the next crop wrote A's template into B's config.
    await waitFor(() =>
      expect(screen.queryByRole("img", { name: "Khung hình thiết bị" })).toBeNull(),
    );
    expect(onConfigFor).not.toHaveBeenCalled();
  });

  it("ignores a device frame that arrives after the selection moved", async () => {
    const gate: { release?: (frame: FlowCoordinateFrame) => void } = {};
    const onConfigFor = vi.fn();
    render(
      <SwitchingHarness
        loadCoordinateFrame={() =>
          new Promise<FlowCoordinateFrame>((resolve) => {
            gate.release = resolve;
          })
        }
        onConfigFor={onConfigFor}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Chụp mẫu từ thiết bị" }));
    await waitFor(() => expect(gate.release).toBeDefined());
    fireEvent.click(screen.getByRole("button", { name: "chọn node B" }));

    gate.release?.(FRAME);
    await waitFor(() => expect(gate.release).toBeDefined());
    expect(screen.queryByRole("img", { name: "Khung hình thiết bị" })).toBeNull();
    expect(onConfigFor).not.toHaveBeenCalled();
  });
});

describe("FlowVisionCapture", () => {
  /**
   * The decode and the canvas have to be stubbed, or these cases prove nothing.
   *
   * jsdom never fires `onload` for a `data:` image and its `getContext("2d")` returns null, so the
   * crop promise either never settles or rejects — and `onCapture` is not called whether the guard
   * is there or not. Written that way, the first version of these two tests passed against the
   * broken code, which is the one outcome a regression test must never have.
   */
  let decode: (() => void) | null = null;

  beforeEach(() => {
    decode = null;
    class DecodingImage {
      onload: (() => void) | null = null;
      onerror: (() => void) | null = null;
      set src(_value: string) {
        decode = () => this.onload?.();
      }
    }
    vi.stubGlobal("Image", DecodingImage);
    HTMLCanvasElement.prototype.getContext = vi.fn(() => ({ drawImage: vi.fn() })) as never;
    HTMLCanvasElement.prototype.toDataURL = vi.fn(() => "data:image/png;base64,QUJD");
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  function pick(image: HTMLElement, clientX: number, clientY: number) {
    // `projectContainedImageClick` needs a measured rectangle; jsdom reports zeros.
    image.getBoundingClientRect = () =>
      ({ left: 0, top: 0, width: 400, height: 800, right: 400, bottom: 800, x: 0, y: 0 }) as DOMRect;
    fireEvent.click(image, { clientX, clientY });
  }

  it("does not capture after Hủy, even though the crop had already started", async () => {
    // Cancel used to call `onCancel` and nothing else, so a decode that finished afterwards still
    // applied the template and region to whatever node the inspector was showing by then.
    const onCapture = vi.fn();
    const onCancel = vi.fn();
    render(<FlowVisionCapture frame={FRAME} onCapture={onCapture} onCancel={onCancel} />);
    const image = screen.getByRole("img", { name: "Khung hình thiết bị" });

    pick(image, 10, 10);
    pick(image, 200, 400);
    expect(decode).not.toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Hủy" }));
    expect(onCancel).toHaveBeenCalledTimes(1);

    // Only now does the frame finish decoding.
    await act(async () => {
      decode?.();
      await Promise.resolve();
    });
    expect(onCapture).not.toHaveBeenCalled();
  });

  it("captures once when the crop completes and the capture is still live", async () => {
    const onCapture = vi.fn();
    render(<FlowVisionCapture frame={FRAME} onCapture={onCapture} onCancel={vi.fn()} />);
    const image = screen.getByRole("img", { name: "Khung hình thiết bị" });

    pick(image, 10, 10);
    pick(image, 200, 400);
    await act(async () => {
      decode?.();
      await Promise.resolve();
    });
    expect(onCapture).toHaveBeenCalledTimes(1);
    const [, region] = onCapture.mock.calls[0];
    // Screen fractions, taken from the frame's own dimensions.
    expect(region).toEqual({ x0: 10 / 400, y0: 10 / 800, x1: 200 / 400, y1: 400 / 800 });
  });

  it("refuses a second crop while one is still running", async () => {
    // `first` is not cleared after a successful crop (the component expects to be unmounted), so
    // further clicks each started another decode and whichever resolved last decided the template.
    const onCapture = vi.fn();
    render(<FlowVisionCapture frame={FRAME} onCapture={onCapture} onCancel={vi.fn()} />);
    const image = screen.getByRole("img", { name: "Khung hình thiết bị" });

    pick(image, 10, 10);
    pick(image, 200, 400);
    pick(image, 20, 20);
    pick(image, 300, 600);

    await act(async () => {
      decode?.();
      await Promise.resolve();
    });
    expect(onCapture).toHaveBeenCalledTimes(1);
  });

  it("keeps crop failures out of the operator-facing message", async () => {
    HTMLCanvasElement.prototype.getContext = vi.fn(() => null);
    render(<FlowVisionCapture frame={FRAME} onCapture={vi.fn()} onCancel={vi.fn()} />);
    const image = screen.getByRole("img", { name: "Khung hình thiết bị" });

    pick(image, 10, 10);
    pick(image, 200, 400);
    await act(async () => {
      decode?.();
      await Promise.resolve();
    });

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Không thể tạo ảnh mẫu từ vùng đã chọn.");
    expect(screen.getByText("Chi tiết lỗi").closest("details"))
      .toHaveTextContent("canvas 2d context unavailable");
  });
});

describe("FlowDiagnostics", () => {
  it("does not call a document valid while validation is still running", () => {
    // Each edit clears the issue list before the debounced request goes out, so emptiness alone
    // announced "Hợp lệ" over a document that had just lost a required field.
    render(<FlowDiagnostics issues={[]} pending />);
    expect(screen.queryByText("Hợp lệ")).toBeNull();
    expect(screen.getByRole("status")).toBeInTheDocument();
  });

  it("says valid once the answer is in", () => {
    render(<FlowDiagnostics issues={[]} />);
    expect(screen.getByText("Hợp lệ")).toBeInTheDocument();
  });

  it("keeps a raw diagnostic code in technical details instead of the main copy", () => {
    render(<FlowDiagnostics issues={[{
      code: "WaitOutOfRange",
      message: "wait duration exceeds the supported range",
      field: "config.durationMs",
    }]} />);
    expect(screen.queryByText("WaitOutOfRange")).not.toBeInTheDocument();
    expect(screen.getByText("Thời lượng chờ vượt giới hạn.")).toHaveAttribute(
      "title",
      "WaitOutOfRange: wait duration exceeds the supported range",
    );
  });
});

describe("FlowRunDialog", () => {
  function device(udid: string, name: string): DeviceInfo {
    return {
      udid,
      name,
      platform: "android",
      status: "ready",
      model: null,
      osVersion: null,
      note: null,
      groupId: null,
      accountHandle: null,
      lastSeenAt: null,
      streaming: false,
    } as unknown as DeviceInfo;
  }

  it("runs the device the operator can see, after the first one disappears", () => {
    // `oneUdid` was initialised once from `devices[0]` and never reconciled, so when that phone
    // dropped off the list the select visibly fell back to the next option while the state still
    // held the departed one — and Run submitted the invisible choice.
    const onRun = vi.fn();
    const view = render(
      <FlowRunDialog
        devices={[device("A", "Máy A"), device("B", "Máy B")]}
        selectedUdids={[]}
        onRun={onRun}
      />,
    );
    fireEvent.click(screen.getByRole("radio", { name: "Một máy" }));
    expect((screen.getByLabelText("Thiết bị") as HTMLSelectElement).value).toBe("A");

    view.rerender(
      <FlowRunDialog devices={[device("B", "Máy B")]} selectedUdids={[]} onRun={onRun} />,
    );
    expect((screen.getByLabelText("Thiết bị") as HTMLSelectElement).value).toBe("B");

    fireEvent.click(screen.getByRole("button", { name: "Chạy trên thiết bị" }));
    expect(onRun).toHaveBeenCalledWith({ mode: "one", udid: "B" });
  });
});

describe("the canvas summary of a vision node", () => {
  it("shows the threshold that is stored, not a rounded one", () => {
    // `toFixed(2)` turned a stored 0.854 into "0.85", so a match score of 0.852 failed while the
    // summary on the canvas implied it passed.
    expect(summarizeAction("tapVision", { templatePngBase64: "AA", threshold: 0.854 }))
      .toContain("0.854");
    expect(summarizeAction("tapVision", { templatePngBase64: "AA", threshold: 0.85 }))
      .toContain("0.85");
  });

  it("uses Vietnamese summaries for operator-visible node data", () => {
    expect(summarizeAction("swipe", { durationMs: 350 })).toBe("Vuốt 350 ms");
    expect(summarizeAction("typeText", { text: "xin chào" })).toBe("8 ký tự");
    expect(summarizeAction("tapVision", { threshold: 0.85 })).toBe("chưa có ảnh mẫu");
  });
});

describe("evidence numbers stay inside what the wire type can hold", () => {
  /** A Tap node with a frame-region postcondition, which is where all five u32 fields live. */
  function tapWithRegion(onEvidence: (spec: EvidenceSpec | null) => void) {
    const node: FlowNode = {
      id: "node-tap",
      kind: "tap",
      position: { x: 0, y: 0 },
      config: {},
      postcondition: {
        kind: "frameRegionChanged",
        x: 4,
        y: 8,
        width: 20,
        height: 30,
        minimumDistance: 2,
      },
    };
    const action: ActionDefinition = {
      ...definition("tap"),
      configSchema: { type: "object", properties: {} },
      evidenceRequirement: "frame",
      allowedEvidence: ["frameRegionChanged"],
    };
    render(
      <FlowInspector
        node={node}
        definition={action}
        issues={[]}
        onConfigChange={() => undefined}
        onPostconditionChange={onEvidence}
      />,
    );
  }

  it.each([
    ["a fraction", "0.5"],
    ["a negative", "-1"],
  ])("refuses %s where Rust declares u32", (_label, typed) => {
    // `x`, `y`, `width`, `height` and `minimumDistance` are all `u32` in `EvidenceSpec`. The input
    // used `step="any"` with no minimum, so these were accepted, stored in the draft, and then
    // failed *deserialization* at the Tauri command boundary -- a whole-document refusal with
    // nothing naming the field the operator had typed in.
    const onEvidence = vi.fn();
    tapWithRegion(onEvidence);
    fireEvent.change(screen.getByLabelText("Khoảng cách tối thiểu"), {
      target: { value: typed },
    });
    expect(onEvidence).not.toHaveBeenCalled();
  });

  it("still accepts a whole number", () => {
    const onEvidence = vi.fn();
    tapWithRegion(onEvidence);
    fireEvent.change(screen.getByLabelText("Khoảng cách tối thiểu"), { target: { value: "7" } });
    expect(onEvidence).toHaveBeenCalledWith(
      expect.objectContaining({ kind: "frameRegionChanged", minimumDistance: 7 }),
    );
  });
});
