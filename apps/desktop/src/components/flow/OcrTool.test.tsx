import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { OcrRequest, OcrResponse } from "../../api";
import type { FlowCoordinateFrame } from "../../types";
import { OcrTool } from "./OcrTool";

const ocr = vi.hoisted(() => vi.fn());
vi.mock("../../api", () => ({ guiOcr: ocr }));
const frame: FlowCoordinateFrame = { jpegBase64: btoa("image"), imageWidth: 900, imageHeight: 280, orientation: "portrait", profileId: "fixture" };
const props = { frame, roi: null, minConfidence: 0.7, languages: ["vi", "en"] as ("vi" | "en")[] };
function response(request: OcrRequest): OcrResponse {
  return { ...request, screenshotSha256: request.screenshot.sha256, status: "resolved", text: "Xin chào Việt Nam", engine: "Tesseract",
    elapsedMs: 210, lines: [{ text: "Xin chào Việt Nam", bounds: { x: 30, y: 40, width: 300, height: 38 }, confidence: 0.96 }] };
}
beforeEach(() => {
  ocr.mockReset();
  vi.stubGlobal("crypto", { randomUUID: () => "session", subtle: { digest: async () => new Uint8Array(32).fill(4).buffer } });
});
afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

it("reads only the captured image and displays Vietnamese text, confidence, and bounds", async () => {
  ocr.mockImplementation(async (request: OcrRequest) => response(request));
  render(<OcrTool {...props} />);
  expect(ocr).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Kiểm tra OCR" }));
  expect(await screen.findByLabelText("Nội dung OCR")).toHaveValue("Xin chào Việt Nam");
  expect(ocr.mock.calls[0][0]).toMatchObject({ minConfidence: 0.7, languages: ["vi", "en"], roi: null, remainingMs: 30000 });
  expect(ocr.mock.calls[0][0].screenshot.sha256).toMatch(/^[a-f0-9]{64}$/);
  expect(screen.getByRole("status")).toHaveTextContent("96.0%");
  expect(document.querySelector("svg rect")).toHaveAttribute("x", "30");
});

it("does not apply a result from an old frame or old confidence threshold", async () => {
  let finish: (() => void) | undefined;
  ocr.mockImplementation((request: OcrRequest) => new Promise((resolve) => { finish = () => resolve(response(request)); }));
  const view = render(<OcrTool {...props} />);
  fireEvent.click(screen.getByRole("button", { name: "Kiểm tra OCR" }));
  await waitFor(() => expect(ocr).toHaveBeenCalledTimes(1));
  view.rerender(<OcrTool {...props} minConfidence={0.95} />);
  finish?.();
  await waitFor(() => expect(screen.getByRole("button", { name: "Kiểm tra OCR" })).toBeEnabled());
  expect(screen.queryByLabelText("Nội dung OCR")).toBeNull();
});

it("keeps missing models or service errors explicit without inventing OCR text", async () => {
  ocr.mockRejectedValue(new Error("gui_ocr_http_503"));
  render(<OcrTool {...props} />);
  fireEvent.click(screen.getByRole("button", { name: "Kiểm tra OCR" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("gui_ocr_http_503");
  expect(screen.queryByLabelText("Nội dung OCR")).toBeNull();
});
