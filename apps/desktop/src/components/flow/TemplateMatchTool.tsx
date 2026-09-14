import { useEffect, useRef, useState } from "react";
import { guiTemplateMatch, type TemplateMatchResponse, type TemplatePixelRect } from "../../api";
import { describeError } from "../../describeError";
import type { FlowCoordinateFrame } from "../../types";

async function imageSha256(base64: string): Promise<string> {
  const binary = atob(base64);
  const bytes = Uint8Array.from(binary, (character) => character.charCodeAt(0));
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

/** Test a crop against an already captured frame. No phone IPC is used here. */
export function TemplateMatchTool({ frame, templatePngBase64, crop }: {
  frame: FlowCoordinateFrame;
  templatePngBase64: string;
  crop: TemplatePixelRect;
}) {
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<TemplateMatchResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [threshold, setThreshold] = useState(0.9);
  const [multiScale, setMultiScale] = useState(false);
  const [limitRegion, setLimitRegion] = useState(false);
  const generation = useRef(0);
  const [session] = useState(() => crypto.randomUUID());
  const pending = useRef(false);

  useEffect(() => {
    generation.current += 1;
    pending.current = false;
    setBusy(false);
    setResult(null);
    setError(null);
    return () => { generation.current += 1; };
  }, [frame.jpegBase64, frame.imageWidth, frame.imageHeight, templatePngBase64, crop.x, crop.y, crop.width, crop.height]);

  const run = async () => {
    if (pending.current) return;
    pending.current = true;
    const ticket = ++generation.current;
    setBusy(true);
    setResult(null);
    setError(null);
    try {
      const [screenshotHash, templateHash] = await Promise.all([
        imageSha256(frame.jpegBase64), imageSha256(templatePngBase64),
      ]);
      if (generation.current !== ticket) return;
      const response = await guiTemplateMatch({
        protocolVersion: 1,
        requestId: crypto.randomUUID(),
        observationId: `capture-${screenshotHash}`,
        sessionEpoch: session,
        generation: ticket,
        remainingMs: 30000,
        screenshot: { bytesBase64: frame.jpegBase64, sha256: screenshotHash, width: frame.imageWidth, height: frame.imageHeight },
        template: { bytesBase64: templatePngBase64, sha256: templateHash, width: crop.width, height: crop.height },
        roi: limitRegion ? crop : null,
        scales: multiScale ? [0.75, 1, 1.25, 1.5] : [1],
        threshold,
      });
      if (generation.current === ticket) setResult(response);
    } catch (cause) {
      if (generation.current === ticket) setError(describeError(cause));
    } finally {
      if (generation.current === ticket) {
        pending.current = false;
        setBusy(false);
      }
    }
  };

  return (
    <div className="flow-template-test">
      <svg className="flow-template-test-frame" viewBox={`0 0 ${frame.imageWidth} ${frame.imageHeight}`} role="img" aria-label="Vị trí tìm thấy trên ảnh đã chụp">
        <image href={`data:image/jpeg;base64,${frame.jpegBase64}`} width={frame.imageWidth} height={frame.imageHeight} />
        {result?.candidates.map((candidate, index) => (
          <rect key={`${candidate.bounds.x}-${candidate.bounds.y}-${index}`} {...candidate.bounds}
            fill="none" stroke={result.status === "resolved" ? "#16a34a" : "#d97706"}
            strokeWidth={3} vectorEffect="non-scaling-stroke" />
        ))}
      </svg>
      <p>Kiểm tra trên ảnh đã chụp. Kết quả áp dụng cho ảnh này; khi giao diện đổi, hãy chụp và kiểm tra lại.</p>
      <p>Ngưỡng và tỷ lệ dưới đây chỉ dùng thử mẫu, không thay cấu hình chạy Flow.</p>
      <fieldset disabled={busy}>
        <legend>Phạm vi kiểm tra ảnh mẫu</legend>
        <label>Ngưỡng khớp
          <input type="number" min={0.5} max={1} step={0.01} value={threshold}
            onChange={(event) => {
              const value = event.currentTarget.valueAsNumber;
              if (Number.isFinite(value) && value >= 0.5 && value <= 1) { setThreshold(value); setResult(null); }
            }} />
        </label>
        <label><input type="checkbox" checked={multiScale}
          onChange={(event) => { setMultiScale(event.currentTarget.checked); setResult(null); }} />
          Thử nhiều tỷ lệ (75%, 100%, 125%, 150%)
        </label>
        <label><input type="checkbox" checked={limitRegion}
          onChange={(event) => { setLimitRegion(event.currentTarget.checked); setResult(null); }} />
          Chỉ tìm trong vùng vừa chọn
        </label>
      </fieldset>
      <button type="button" disabled={busy} onClick={() => void run()}>
        {busy ? "Đang kiểm tra ảnh mẫu…" : "Kiểm tra ảnh mẫu"}
      </button>
      {result && <div role="status">
        <strong>{result.status === "resolved" ? "Tìm thấy một vị trí" : result.status === "ambiguous" ? "Nhiều vị trí giống nhau — cần chọn mẫu rõ hơn" : "Chưa tìm thấy mẫu đủ rõ"}</strong>
        <p>{result.elapsedMs} ms · {result.searchedScales.length} tỷ lệ đã kiểm tra</p>
        {result.candidates.map((candidate, index) => <p key={index}>
          Vị trí {index + 1}: ({candidate.bounds.x}, {candidate.bounds.y}) · {candidate.bounds.width} × {candidate.bounds.height} px · độ khớp {candidate.score.toFixed(4)} · tỷ lệ {candidate.scale}
        </p>)}
      </div>}
      {error && <div role="alert">Chưa kiểm tra được ảnh mẫu.<details><summary>Chi tiết lỗi</summary><code>{error}</code></details></div>}
    </div>
  );
}
