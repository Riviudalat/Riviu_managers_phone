import { useEffect, useState } from "react";
import { nurtureTestApi } from "../../api";
import { exportViewJpegBurst } from "../../viewStore";
import { IconApi } from "../Icons";
import type { DeviceInfo, NurtureApiTestResult, NurtureSettings } from "../../types";
import { describeError } from "../../describeError";
import { InfoDot as Info } from "../InfoDot";

/**
 * Where the comments come from: endpoint, model, key, price, and the test that proves it.
 *
 * Owns the API-test state, which is the one cluster in this panel that belongs to a single
 * tab — the other two tabs are functions of `settings` and nothing else.
 */
export function NurtureAiTab({ settings, patch, devices, targets, save, onMessage }: {
  settings: NurtureSettings;
  patch: <K extends keyof NurtureSettings>(key: K, value: NurtureSettings[K]) => void;
  devices: DeviceInfo[];
  /// The phones this panel is acting on; the first is the default to test against.
  targets: string[];
  /// Persist before testing — the endpoint must be the one just typed.
  save: (next?: NurtureSettings) => Promise<boolean>;
  onMessage: (text: string | null) => void;
}) {
  const [apiTesting, setApiTesting] = useState(false);
  const [apiTest, setApiTest] = useState<NurtureApiTestResult | null>(null);
  const [testUdid, setTestUdid] = useState("");
  const fallbackTestUdid = targets[0] ?? devices[0]?.udid ?? "";

  // Keep the picker on a phone that still exists: unplugging the one it named used to leave
  // the field pointing at nothing, and the test then failed on a udid the fleet had lost.
  useEffect(() => {
    setTestUdid((current) => {
      if (current && devices.some((device) => device.udid === current)) return current;
      return fallbackTestUdid;
    });
  }, [devices, fallbackTestUdid]);

  const runApiTest = async () => {
    const udid = testUdid || fallbackTestUdid;
    if (!udid) {
      onMessage("Chọn một máy trước khi test API");
      return;
    }
    setApiTesting(true);
    setApiTest(null);
    onMessage(null);
    try {
      if (!(await save(settings))) return;
      setApiTest(await nurtureTestApi(udid, await exportViewJpegBurst(udid)));
    } catch (e) {
      onMessage(describeError(e));
    } finally {
      setApiTesting(false);
    }
  };

  return (
    <div className="nurture-sect" role="tabpanel">
      <label>
        <span className="nu-inline">
          Base URL
          <Info
            of="Base URL"
            what="Endpoint tương thích OpenAI dùng để sinh bình luận. Đổi được trong lúc phiên đang chạy, áp từ bình luận kế tiếp."
          />
        </span>
        <input value={settings.baseUrl} onChange={(e) => patch("baseUrl", e.target.value)} />
      </label>
      <label>
        <span className="nu-inline">
          Model
          <Info of="Model" what="Tên model gửi kèm mỗi lần gọi endpoint ở trên." />
        </span>
        <input value={settings.model} onChange={(e) => patch("model", e.target.value)} />
      </label>
      <label>
        <span className="nu-inline">
          API key
          <Info
            of="API key"
            what="Khoá của endpoint ở trên. Khoá được cất trong kho mật khẩu của hệ điều hành, không nằm trong file dữ liệu — nên panel này không hiện lại khoá đã lưu, chỉ báo là đã có. Gõ khoá mới để thay, xoá trắng ô rồi lưu để bỏ hẳn."
          />
        </span>
        <input
          type="password"
          value={settings.apiKey}
          onChange={(e) => patch("apiKey", e.target.value)}
          autoComplete="off"
          placeholder={settings.hasApiKey ? "Đã lưu — gõ để thay" : "Chưa có khoá"}
        />
      </label>
      <div className="nurture-row">
        <label>
          <span className="nu-inline">
            Ngôn ngữ
            <Info of="Ngôn ngữ" what="Ngôn ngữ AI viết bình luận." />
          </span>
          <select
            value={settings.commentLang || "vi"}
            onChange={(e) => patch("commentLang", e.target.value)}
          >
            <option value="vi">Tiếng Việt</option>
            <option value="en">English</option>
          </select>
        </label>
        <label>
          <span className="nu-inline">
            Tối đa từ
            <Info
              of="Tối đa từ"
              what="Chặn độ dài bình luận. Bình luận dài dễ lộ là máy viết, và cũng làm việc đọc lại để tìm comment cha khó hơn."
            />
          </span>
          <input
            type="number"
            min={4}
            max={30}
            value={settings.maxCommentWords}
            onChange={(e) => patch("maxCommentWords", Number(e.target.value) || 4)}
          />
        </label>
      </div>
      <label>
        <span className="nu-inline">
          Định hướng giọng điệu
          <Info
            of="Định hướng giọng điệu"
            what="Mô tả giọng muốn AI viết theo, nhiều lựa chọn cách nhau bằng dấu | và mỗi bình luận lấy ngẫu nhiên một cái."
          />
        </span>
        <input
          value={settings.aiDirections}
          onChange={(e) => patch("aiDirections", e.target.value)}
          placeholder="Tự nhiên|Thân thiện|Hơi hài"
        />
      </label>
      <div className="nurture-api-test">
        <div className="nurture-api-test-row">
          <label>
            Thiết bị test
            <select
              value={testUdid || fallbackTestUdid}
              onChange={(event) => setTestUdid(event.target.value)}
              disabled={!devices.length || apiTesting}
            >
              {!devices.length && <option value="">Chưa có thiết bị</option>}
              {devices.map((device) => (
                <option key={device.udid} value={device.udid}>
                  {device.name || device.udid.slice(0, 8)}
                </option>
              ))}
            </select>
          </label>
          <button
            type="button"
            className="primary nurture-api-test-button"
            disabled={!devices.length || apiTesting}
            onClick={() => void runApiTest()}
            title="Gọi API trên frame hiện tại, không gửi comment"
          >
            <IconApi size={14} />
            {apiTesting ? "Đang test…" : "Test API"}
          </button>
        </div>
        <p className="hint">Preview comment từ frame hiện tại · không gửi lên TikTok</p>
        {apiTest && (
          <div className="nurture-api-result" aria-live="polite">
            <strong>Comment trả về</strong>
            <p className="nurture-api-result-comment">“{apiTest.comment}”</p>
            <p className="nurture-api-result-meta">
              {apiTest.model} · {apiTest.baseUrlHost} · {apiTest.evidenceMode === "ocr-caption" ? "OCR caption + text" : "3-frame vision"} · {apiTest.promptTokens + apiTest.completionTokens} tokens · ${apiTest.usd.toFixed(5)}
            </p>
            <p className="hint">
              Context {apiTest.contextConfidence}/100 · liên quan {apiTest.relevance}/100 · bằng chứng {apiTest.evidenceSupport}/100
              {apiTest.caption ? ` · caption: ${apiTest.caption}` : ""}
            </p>
          </div>
        )}
      </div>
    </div>
  );
}
