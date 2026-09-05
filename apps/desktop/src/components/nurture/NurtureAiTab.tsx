import { useEffect, useState } from "react";
import { nurtureTestApi } from "../../api";
import { exportViewJpegBurst } from "../../viewStore";
import { IconApi } from "../Icons";
import type { DeviceInfo, NurtureApiTestResult, NurtureSettings } from "../../types";
import { describeError } from "../../describeError";
import { COMMENT_MODEL_SUGGESTIONS } from "../../commentModels";
import { evidenceLabel } from "../../commentEvidence";
import { InfoDot as Info } from "../InfoDot";
import { nurtureFieldValidation, type NurtureSettingsIssue } from "../../nurtureValidation";

/**
 * Where the comments come from: endpoint, model, key, price, and the test that proves it.
 *
 * Owns the API-test state, which is the one cluster in this panel that belongs to a single
 * tab — the other two tabs are functions of `settings` and nothing else.
 */
export function NurtureAiTab({ settings, patch, devices, targets, save, onMessage, issue, issueId }: {
  settings: NurtureSettings;
  issue?: NurtureSettingsIssue | null;
  issueId?: string;
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
    <div className="nurture-sect">
      <label>
        <span className="nu-inline">
          Địa chỉ API
          <Info
            of="Địa chỉ API"
            what="Endpoint tương thích OpenAI dùng để sinh bình luận. Đổi được trong lúc phiên đang chạy, áp từ bình luận kế tiếp."
          />
        </span>
        <input
          value={settings.baseUrl}
          list="riviu-comment-base-urls"
          onChange={(e) => patch("baseUrl", e.target.value)}
        />
        <datalist id="riviu-comment-base-urls">
          {[...new Set(COMMENT_MODEL_SUGGESTIONS.map((s) => s.baseUrl))].map((url) => (
            <option key={url} value={url} />
          ))}
        </datalist>
      </label>
      <label>
        <span className="nu-inline">
          Mô hình
          <Info
            of="Mô hình"
            what="Tên model gửi kèm mỗi lần gọi endpoint ở trên. Bất kỳ endpoint tương thích OpenAI nào cũng chạy — danh sách gợi ý bên dưới chỉ là những cái dự án này đã đo thật."
          />
        </span>
        {/* A `datalist`, not a `select`: the client speaks plain OpenAI-compatible
            `chat/completions`, so any model string works and locking the field to a list
            would be inventing a restriction the code does not have. */}
        <input
          value={settings.model}
          list="riviu-comment-models"
          onChange={(e) => patch("model", e.target.value)}
        />
        <datalist id="riviu-comment-models">
          {COMMENT_MODEL_SUGGESTIONS.map((s) => (
            <option key={s.model} value={s.model} />
          ))}
        </datalist>
      </label>
      <label>
        <span className="nu-inline">
          Khóa API
          <Info
            of="Khóa API"
            what="Khoá của endpoint ở trên. Khoá được cất trong kho mật khẩu của hệ điều hành, không nằm trong file dữ liệu — nên panel này không hiện lại khoá đã lưu, chỉ báo là đã có. Gõ khoá mới để thay, xoá trắng ô rồi lưu để bỏ hẳn."
          />
        </span>
        <input
          type="password"
          value={settings.apiKey}
          {...nurtureFieldValidation("apiKey", issue, issueId)}
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
            <option value="en">Tiếng Anh</option>
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
            {...nurtureFieldValidation("maxCommentWords", issue, issueId)}
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
            Thiết bị kiểm thử
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
            title="Kiểm tra bằng khung hình hiện tại, không gửi bình luận"
          >
            <IconApi size={14} />
            {apiTesting ? "Đang kiểm tra…" : "Kiểm tra API"}
          </button>
        </div>
        <p className="hint">Xem trước từ khung hình hiện tại · không gửi lên TikTok</p>
        {apiTest && (
          <div className="nurture-api-result" aria-live="polite">
            <strong>Bình luận đề xuất</strong>
            <p className="nurture-api-result-comment">“{apiTest.comment}”</p>
            <p className="nurture-api-result-meta">
              {apiTest.model} · {apiTest.baseUrlHost} · {evidenceLabel(apiTest)} · {apiTest.promptTokens} token vào / {apiTest.completionTokens} ra
            </p>
            <p className="hint">
              Ngữ cảnh {apiTest.contextConfidence}/100 · liên quan {apiTest.relevance}/100 · bằng chứng {apiTest.evidenceSupport}/100
              {apiTest.caption ? ` · chú thích: ${apiTest.caption}` : ""}
            </p>
          </div>
        )}
      </div>
    </div>
  );
}
