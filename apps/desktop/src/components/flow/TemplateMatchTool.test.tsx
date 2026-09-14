import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { TemplateMatchRequest, TemplateMatchResponse } from "../../api";
import type { FlowCoordinateFrame } from "../../types";
import { TemplateMatchTool } from "./TemplateMatchTool";

const match = vi.hoisted(() => vi.fn());
vi.mock("../../api", () => ({ guiTemplateMatch: match }));

const frame: FlowCoordinateFrame = {
  jpegBase64: btoa("screenshot-fixture"), imageWidth: 400, imageHeight: 800,
  orientation: "portrait", profileId: "fixture",
};
const crop = { x: 20, y: 30, width: 40, height: 50 };
const props = { frame, templatePngBase64: btoa("template-fixture"), crop };

function result(request: TemplateMatchRequest, status: TemplateMatchResponse["status"] = "resolved"): TemplateMatchResponse {
  const candidate = { bounds: crop, score: 0.9876, scale: 1, method: "templateCorrelation" as const };
  return {
    ...request, screenshotSha256: request.screenshot.sha256, templateSha256: request.template.sha256,
    status, reason: "fixture", elapsedMs: 42, searchedScales: [1],
    candidates: status === "unresolved" ? [] : status === "resolved" ? [candidate] : [candidate, { ...candidate, bounds: { ...crop, x: 100 } }],
  };
}

beforeEach(() => {
  match.mockReset();
  vi.stubGlobal("crypto", {
    randomUUID: () => "fixture-uuid",
    subtle: { digest: vi.fn(async (algorithm: string, bytes: Uint8Array) => {
      expect(algorithm).toBe("SHA-256");
      return new Uint8Array(32).fill(bytes[0]).buffer;
    }) },
  });
});
afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

describe("TemplateMatchTool", () => {
  it("hashes both captured images and explicitly invokes one image-only request", async () => {
    match.mockImplementation(async (request: TemplateMatchRequest) => result(request));
    render(<TemplateMatchTool {...props} />);
    expect(match).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Kiểm tra ảnh mẫu" }));
    await screen.findByText("Tìm thấy một vị trí");
    expect(match).toHaveBeenCalledTimes(1);
    const request: TemplateMatchRequest = match.mock.calls[0][0];
    expect(request).toMatchObject({ protocolVersion: 1, roi: null, scales: [1], threshold: 0.9, remainingMs: 30000 });
    expect(request.screenshot).toMatchObject({ bytesBase64: frame.jpegBase64, width: 400, height: 800 });
    expect(request.template).toMatchObject({ width: 40, height: 50 });
    expect(request.screenshot.sha256).toMatch(/^[a-f0-9]{64}$/);
    expect(request.template.sha256).not.toBe(request.screenshot.sha256);
    expect(request.observationId).toBe(`capture-${request.screenshot.sha256}`);
    expect(screen.getByRole("status")).toHaveTextContent("0.9876");
    expect(document.querySelector("svg rect")).toHaveAttribute("x", "20");
  });

  it("keeps multi-scale and ROI inspection explicit and clears outdated results after option edits", async () => {
    match.mockImplementation(async (request: TemplateMatchRequest) => result(request, "ambiguous"));
    render(<TemplateMatchTool {...props} />);
    fireEvent.click(screen.getByLabelText("Thử nhiều tỷ lệ (75%, 100%, 125%, 150%)"));
    fireEvent.click(screen.getByLabelText("Chỉ tìm trong vùng vừa chọn"));
    fireEvent.click(screen.getByRole("button", { name: "Kiểm tra ảnh mẫu" }));
    await screen.findByText("Nhiều vị trí giống nhau — cần chọn mẫu rõ hơn");
    expect(match.mock.calls[0][0]).toMatchObject({ roi: crop, scales: [0.75, 1, 1.25, 1.5] });
    expect(document.querySelectorAll("svg rect")).toHaveLength(2);
    fireEvent.change(screen.getByLabelText("Ngưỡng khớp"), { target: { value: "0.95" } });
    expect(screen.queryByRole("status")).toBeNull();
    expect(document.querySelectorAll("svg rect")).toHaveLength(0);
  });

  it("does not reuse an old crop result after the capture changes", async () => {
    let finish: (() => void) | undefined;
    match.mockImplementation((request: TemplateMatchRequest) => new Promise<TemplateMatchResponse>((resolve) => {
      finish = () => resolve(result(request));
    }));
    const view = render(<TemplateMatchTool {...props} />);
    fireEvent.click(screen.getByRole("button", { name: "Kiểm tra ảnh mẫu" }));
    await waitFor(() => expect(match).toHaveBeenCalledTimes(1));
    expect(screen.getByRole("button", { name: "Đang kiểm tra ảnh mẫu…" })).toBeDisabled();
    view.rerender(<TemplateMatchTool {...props} templatePngBase64={btoa("new-crop")} />);
    finish?.();
    await waitFor(() => expect(screen.getByRole("button", { name: "Kiểm tra ảnh mẫu" })).toBeEnabled());
    expect(screen.queryByText("Tìm thấy một vị trí")).toBeNull();
  });

  it("shows missing and transport failures without inventing match coordinates", async () => {
    match.mockImplementationOnce(async (request: TemplateMatchRequest) => result(request, "unresolved"));
    match.mockRejectedValueOnce(new Error("gui_template_http_429"));
    render(<TemplateMatchTool {...props} />);
    fireEvent.click(screen.getByRole("button", { name: "Kiểm tra ảnh mẫu" }));
    await screen.findByText("Chưa tìm thấy mẫu đủ rõ");
    expect(document.querySelectorAll("svg rect")).toHaveLength(0);
    fireEvent.click(screen.getByRole("button", { name: "Kiểm tra ảnh mẫu" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Chưa kiểm tra được ảnh mẫu.");
    expect(screen.queryByRole("status")).toBeNull();
  });
});
