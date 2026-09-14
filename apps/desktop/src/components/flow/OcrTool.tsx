import { useEffect, useRef, useState } from "react";
import { guiOcr, type OcrResponse, type TemplatePixelRect } from "../../api";
import { describeError } from "../../describeError";
import type { FlowCoordinateFrame } from "../../types";

export function OcrTool({ frame, roi, minConfidence, languages }: {
  frame: FlowCoordinateFrame;
  roi: TemplatePixelRect | null;
  minConfidence: number;
  languages: ("vi" | "en")[];
}) {
  const [result, setResult] = useState<OcrResponse | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const generation = useRef(0);
  const pending = useRef(false);
  const [session] = useState(() => crypto.randomUUID());
  const languageKey = languages.join(",");

  useEffect(() => {
    generation.current += 1;
    pending.current = false;
    setResult(null);
    setError(null);
    setBusy(false);
    return () => { generation.current += 1; };
  }, [frame.jpegBase64, frame.imageWidth, frame.imageHeight, roi?.x, roi?.y, roi?.width, roi?.height, minConfidence, languageKey]);

  const run = async () => {
    if (pending.current) return;
    pending.current = true;
    const ticket = ++generation.current;
    setBusy(true);
    setResult(null);
    setError(null);
    try {
      const bytes = Uint8Array.from(atob(frame.jpegBase64), (character) => character.charCodeAt(0));
      const digest = await crypto.subtle.digest("SHA-256", bytes);
      const sha256 = [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
      if (ticket !== generation.current) return;
      const response = await guiOcr({
        protocolVersion: 1, requestId: crypto.randomUUID(), observationId: `capture-${sha256}`,
        sessionEpoch: session, generation: ticket, remainingMs: 30000,
        screenshot: { bytesBase64: frame.jpegBase64, sha256, width: frame.imageWidth, height: frame.imageHeight },
        roi, minConfidence, languages,
      });
      if (ticket === generation.current) setResult(response);
    } catch (cause) {
      if (ticket === generation.current) setError(describeError(cause));
    } finally {
      if (ticket === generation.current) { pending.current = false; setBusy(false); }
    }
  };

  return <div className="flow-template-test flow-ocr-test">
    <svg className="flow-template-test-frame" viewBox={`0 0 ${frame.imageWidth} ${frame.imageHeight}`} role="img" aria-label="Vị trí chữ OCR trên ảnh đã chụp">
      <image href={`data:image/jpeg;base64,${frame.jpegBase64}`} width={frame.imageWidth} height={frame.imageHeight} />
      {roi && <rect {...roi} fill="none" stroke="#d97706" strokeDasharray="8 4" strokeWidth={2} vectorEffect="non-scaling-stroke" />}
      {result?.lines.map((line, index) => <rect key={index} {...line.bounds} fill="none" stroke="#16a34a" strokeWidth={2} vectorEffect="non-scaling-stroke"><title>{line.text}</title></rect>)}
    </svg>
    <p>Đọc chữ ngay trên máy tính từ ảnh đã chụp. Flow dùng cùng vùng và ngưỡng tin cậy này trên ảnh mới của mỗi lượt chạy.</p>
    <button type="button" disabled={busy || languages.length === 0} onClick={() => void run()}>
      {busy ? "Đang đọc chữ từ ảnh…" : "Kiểm tra OCR"}
    </button>
    {result && <div role="status">
      <strong>{result.status === "resolved" ? "Đã đọc được chữ" : "Chưa tìm thấy chữ đủ rõ"}</strong>
      <p>{result.elapsedMs} ms · {result.lines.length} dòng</p>
      <textarea aria-label="Nội dung OCR" readOnly value={result.text} rows={Math.min(8, Math.max(2, result.lines.length))} />
      {result.lines.map((line, index) => <p key={index}>{line.text} · {(line.confidence * 100).toFixed(1)}% · ({line.bounds.x}, {line.bounds.y})</p>)}
    </div>}
    {error && <div role="alert">Chưa đọc được chữ từ ảnh.<details><summary>Chi tiết lỗi</summary><code>{error}</code></details></div>}
  </div>;
}
