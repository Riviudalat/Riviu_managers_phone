import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  listenRiviuEvents,
  nurtureGetSettings,
  nurtureSaveSettings,
  nurtureSessionStatus,
  nurtureStart,
  nurtureStop,
  nurtureTestApi,
} from "../api";
import { targetsOf } from "./SelectionStrip";
import { IconApi, IconClose, IconHeart } from "./Icons";
import { LoadingState } from "./States";
import type {
  DeviceInfo,
  NurtureApiTestResult,
  NurtureSessionStatus,
  NurtureSettings,
} from "../types";

type Props = {
  devices: DeviceInfo[];
  selected: string[];
  onClose: () => void;
};

/** Map engine status English → short Vietnamese for the live log. */
function statusVi(raw: string): string {
  const s = raw.trim();
  if (!s) return "—";
  const exact: Record<string, string> = {
    starting: "Đang khởi động…",
    queued: "Đang xếp hàng…",
    stopped: "Đã dừng",
    done: "Xong phiên",
    "ui session": "Mở phiên WDA…",
    "launch TikTok": "Mở TikTok…",
    "ui session: timeout": "WDA timeout — thử lại",
    "clear popups": "Đóng popup",
    like: "Đang thích",
    follow: "Đang follow",
    "comment (vision)": "Đang bình luận (AI)",
    "comment (grounded)": "Đang bình luận (AI đọc nội dung)",
    "swipe next": "Vuốt video tiếp",
    "swipe blocked — clear + retry": "Vuốt kẹt — dọn rồi thử lại",
    "tiktok already open": "TikTok đã mở sẵn — bỏ qua launch",
    "tiktok launched": "Đã mở TikTok",
    "popup: close X": "Phát hiện popup — đóng (nút X)",
    "popup: interest": "Trang chọn chủ đề — bấm Bỏ qua",
    "frenzy scroll": "Vuốt nhanh",
    "off TikTok — relaunch": "Lệch TikTok — mở lại",
    "night window — paused": "Giờ đêm — tạm dừng",
  };
  if (exact[s]) return exact[s];
  if (s.startsWith("watching ")) return `Đang xem ${s.slice("watching ".length)}`;
  if (s.startsWith("round ") && s.includes("ensure TikTok")) return s.replace("round ", "Vòng ").replace(" — ensure TikTok", " — kiểm tra TikTok");
  if (s.startsWith("round ") && s.includes("clear popups")) return s.replace("round ", "Vòng ").replace(" — clear popups", " — dọn popup");
  if (s.startsWith("round ")) return s.replace("round ", "Vòng ").replace(" — open TikTok + clear popups", " — mở TikTok");
  if (s.startsWith("clear onboarding")) return "Đóng popup TikTok (chủ đề / Add phone)…";
  if (s.startsWith("popup: cleared ")) return `Đã đóng ${s.slice("popup: cleared ".length).trim()} popup`;
  if (s.startsWith("recover wait ")) return s.replace("recover wait ", "Khôi phục chờ ").replace("(", " (");
  if (s.startsWith("ensure failed:")) return `Mở TikTok lỗi: ${s.slice("ensure failed:".length).trim()}`;
  if (s.startsWith("like fail:")) return `Thích lỗi: ${s.slice("like fail:".length).trim()}`;
  if (s.startsWith("comment skip:")) return `Bỏ bình luận: ${s.slice("comment skip:".length).trim()}`;
  if (s.startsWith("ui session:")) return `WDA: ${s.slice("ui session:".length).trim()}`;
  if (s.startsWith("error:")) return `Lỗi: ${s.slice("error:".length).trim()}`;
  return s;
}

function deviceLabel(devices: DeviceInfo[], udid: string): string {
  const d = devices.find((x) => x.udid === udid);
  if (d?.name?.trim()) return d.name.trim();
  return udid.slice(0, 8);
}

export function NurturePopup({ devices, selected, onClose }: Props) {
  const [settings, setSettings] = useState<NurtureSettings | null>(null);
  const [statuses, setStatuses] = useState<NurtureSessionStatus[]>([]);
  const [msg, setMsg] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [showAi, setShowAi] = useState(false);
  const [showBeh, setShowBeh] = useState(false);
  const [apiTesting, setApiTesting] = useState(false);
  const [apiTest, setApiTest] = useState<NurtureApiTestResult | null>(null);
  const [testUdid, setTestUdid] = useState("");
  const [pos, setPos] = useState({ x: 0, y: 0 });
  const drag = useRef<{ ox: number; oy: number; sx: number; sy: number } | null>(null);
  const targets = targetsOf(selected, devices);
  const fallbackTestUdid = targets[0] ?? devices[0]?.udid ?? "";
  const anyRunning = statuses.some((s) => s.running);

  useEffect(() => {
    setTestUdid((current) => {
      if (current && devices.some((device) => device.udid === current)) return current;
      return fallbackTestUdid;
    });
  }, [devices, fallbackTestUdid]);

  const totals = useMemo(() => {
    return statuses.reduce(
      (acc, s) => {
        acc.videos += s.videosDone;
        acc.likes += s.likes;
        acc.comments += s.comments;
        acc.follows += s.follows;
        return acc;
      },
      { videos: 0, likes: 0, comments: 0, follows: 0 },
    );
  }, [statuses]);

  const sortedStatuses = useMemo(() => {
    return [...statuses].sort((a, b) => Number(b.running) - Number(a.running));
  }, [statuses]);

  const reload = useCallback(async () => {
    try {
      const [s, st] = await Promise.all([nurtureGetSettings(), nurtureSessionStatus()]);
      setSettings(s);
      setStatuses(st);
      setMsg(null);
    } catch (e) {
      setMsg(String(e));
    }
  }, []);

  useEffect(() => {
    reload();
  }, [reload]);

  useEffect(() => {
    let un: (() => void) | undefined;
    listenRiviuEvents((payload) => {
      const p = payload as { type?: string; status?: NurtureSessionStatus };
      if (p?.type !== "nurtureStatus" || !p.status) return;
      const st = p.status;
      setStatuses((prev) => {
        const next = prev.filter((x) => x.udid !== st.udid);
        next.push(st);
        return next;
      });
    }).then((fn) => {
      un = fn;
    });
    return () => un?.();
  }, []);

  const patch = <K extends keyof NurtureSettings>(key: K, value: NurtureSettings[K]) => {
    setSettings((prev) => (prev ? { ...prev, [key]: value } : prev));
  };

  const actionHint = useMemo(() => {
    if (!settings) return "";
    const none = Math.max(0, 100 - settings.likeProb - settings.commentProb);
    return `Thích ${settings.likeProb}% · Bình luận ${settings.commentProb}% · Bỏ qua ${none}% · Follow độc lập ${settings.followProb}% · Vuốt nhanh ${settings.frenzyProb}%`;
  }, [settings]);

  const save = async (next?: NurtureSettings): Promise<boolean> => {
    const s = next ?? settings;
    if (!s) return false;
    if (s.likeProb + s.commentProb > 100) {
      setMsg(`Thích + Bình luận > 100%`);
      return false;
    }
    if (s.followProb > 100 || s.frenzyProb > 100) {
      setMsg(`Follow và vuốt nhanh phải từ 0 đến 100%`);
      return false;
    }
    if (s.maxCommentWords < 4 || s.maxCommentWords > 30) {
      setMsg(`Giới hạn comment phải từ 4 đến 30 từ`);
      return false;
    }
    if (s.numVideos < 1 || s.numVideos > 10_000 || s.numRounds < 1 || s.numRounds > 100) {
      setMsg(`Giới hạn video phải từ 1 đến 10000 và vòng từ 1 đến 100`);
      return false;
    }
    if (!Number.isFinite(s.watchMin) || !Number.isFinite(s.watchMax) || s.watchMin <= 0 || s.watchMax < s.watchMin || s.watchMax > 120) {
      setMsg(`Khoảng xem phải trong 0 đến 120 giây và min không lớn hơn max`);
      return false;
    }
    if (s.scheduleEveryMinutes < 15 || s.scheduleEveryMinutes > 1440 || s.scheduleDurationMinutes < 15 || s.scheduleDurationMinutes > 360) {
      setMsg(`Lịch phải cách nhau 15–1440 phút và kéo dài 15–360 phút`);
      return false;
    }
    if (s.commentProb > 0 && !s.apiKey.trim()) {
      setMsg(`Đã bật bình luận: điền API key trong Cấu hình AI`);
      return false;
    }
    const payload = {
      ...s,
      scheduleUdids: s.scheduleEnabled ? targets : s.scheduleUdids,
    };
    const saved = await nurtureSaveSettings(payload);
    setSettings(saved);
    return true;
  };

  const start = async () => {
    if (!targets.length) {
      setMsg("Chọn máy trên lưới trước");
      return;
    }
    setBusy(true);
    try {
      if (settings && !(await save({ ...settings, scheduleUdids: targets }))) return;
      await nurtureStart(targets);
      await reload();
    } catch (e) {
      setMsg(String(e));
    } finally {
      setBusy(false);
    }
  };

  const stop = async () => {
    setBusy(true);
    try {
      await nurtureStop(targets.length ? targets : []);
      await reload();
    } catch (e) {
      setMsg(String(e));
    } finally {
      setBusy(false);
    }
  };

  const runApiTest = async () => {
    if (!settings) return;
    const udid = testUdid || fallbackTestUdid;
    if (!udid) {
      setMsg("Chọn một máy có frame stream trước khi test API");
      return;
    }
    setApiTesting(true);
    setApiTest(null);
    setMsg(null);
    try {
      if (!(await save(settings))) return;
      setApiTest(await nurtureTestApi(udid));
    } catch (e) {
      setMsg(String(e));
    } finally {
      setApiTesting(false);
    }
  };

  const onTitleDown = (e: React.PointerEvent<HTMLDivElement>) => {
    if ((e.target as HTMLElement).closest("button")) return;
    drag.current = { ox: e.clientX, oy: e.clientY, sx: pos.x, sy: pos.y };
    e.currentTarget.setPointerCapture(e.pointerId);
  };
  const onTitleMove = (e: React.PointerEvent<HTMLDivElement>) => {
    if (!drag.current) return;
    setPos({
      x: drag.current.sx + (e.clientX - drag.current.ox),
      y: drag.current.sy + (e.clientY - drag.current.oy),
    });
  };
  const onTitleUp = () => {
    drag.current = null;
  };

  return (
    <div className="nurture-float-layer" aria-label="Nuôi TikTok">
      <div
        className="nurture-float"
        style={{ transform: `translate(${pos.x}px, ${pos.y}px)` }}
      >
        <div
          className="nurture-float-title"
          onPointerDown={onTitleDown}
          onPointerMove={onTitleMove}
          onPointerUp={onTitleUp}
        >
          <IconHeart size={14} />
          <strong>Nuôi TikTok</strong>
          <span className="hint">
            {selected.length ? `${selected.length} máy` : `Tất cả ${devices.length}`}
          </span>
          <div className="grow" />
          <button type="button" className="close" title="Đóng" onClick={onClose}>
            <IconClose size={14} />
          </button>
        </div>

        <div className="nurture-float-body">
          {!settings ? (
            msg ? <p className="hint">{msg}</p> : <LoadingState />
          ) : (
            <>
              <div className="nurture-float-actions">
                <button type="button" className="primary" disabled={busy || !devices.length} onClick={start}>
                  Bắt đầu
                </button>
                <button type="button" className="danger" disabled={busy || !anyRunning} onClick={stop}>
                  Dừng
                </button>
                <button
                  type="button"
                  className="ghost"
                  disabled={busy}
                  onClick={async () => {
                    setBusy(true);
                    try {
                      if (await save()) setMsg(null);
                    } catch (e) {
                      setMsg(String(e));
                    } finally {
                      setBusy(false);
                    }
                  }}
                >
                  Lưu
                </button>
              </div>

              {statuses.length > 0 && (
                <>
                  <div className="nurture-float-stats">
                    <div>
                      <span>Video</span>
                      <strong>{totals.videos}</strong>
                    </div>
                    <div>
                      <span>Thích</span>
                      <strong>{totals.likes}</strong>
                    </div>
                    <div>
                      <span>BL / Follow</span>
                      <strong>
                        {totals.comments}/{totals.follows}
                      </strong>
                    </div>
                  </div>
                  <div className="nurture-float-log" aria-live="polite">
                    {sortedStatuses.map((s) => (
                      <div key={s.udid} className={`nurture-float-log-row${s.running ? " is-run" : ""}`}>
                        <div className="nurture-float-log-head">
                          <span className={`nurture-dot${s.running ? " on" : ""}`} />
                          <strong title={s.udid}>{deviceLabel(devices, s.udid)}</strong>
                          <span
                            className="hint"
                            title="video đã xác nhận / lượt vuốt · tim · bình luận · follow"
                          >
                            {s.videosDone}/{s.swipeAttempts}v · ♥{s.likes}/{s.likeAttempts} · BL{s.comments}/{s.commentAttempts} · +{s.follows}/{s.followAttempts}
                          </span>
                        </div>
                        <p className="nurture-float-log-msg">{statusVi(s.lastMessage)}</p>
                      </div>
                    ))}
                  </div>
                </>
              )}
              {msg && <p className="nurture-float-err">{msg}</p>}

              <button type="button" className="nurture-sect-toggle" onClick={() => setShowAi((v) => !v)}>
                {showAi ? "▾" : "▸"} Cấu hình AI
              </button>
              {showAi && (
                <div className="nurture-sect">
                  <label>
                    Base URL
                    <input value={settings.baseUrl} onChange={(e) => patch("baseUrl", e.target.value)} />
                  </label>
                  <label>
                    Model
                    <input value={settings.model} onChange={(e) => patch("model", e.target.value)} />
                  </label>
                  <label>
                    API key
                    <input
                      type="password"
                      value={settings.apiKey}
                      onChange={(e) => patch("apiKey", e.target.value)}
                      autoComplete="off"
                    />
                  </label>
                  <div className="nurture-row">
                    <label>
                      Ngôn ngữ
                      <select
                        value={settings.commentLang || "vi"}
                        onChange={(e) => patch("commentLang", e.target.value)}
                      >
                        <option value="vi">Tiếng Việt</option>
                        <option value="en">English</option>
                      </select>
                    </label>
                    <label>
                      Tối đa từ
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
                    Định hướng giọng điệu
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
              )}

              <button type="button" className="nurture-sect-toggle" onClick={() => setShowBeh((v) => !v)}>
                {showBeh ? "▾" : "▸"} Hành vi
              </button>
              {showBeh && (
                <div className="nurture-sect">
                  <div className="nurture-row">
                    <label>
                      Giới hạn video
                      <input
                        type="number"
                        min={1}
                        max={10000}
                        value={settings.numVideos}
                        onChange={(e) => patch("numVideos", Number(e.target.value) || 1)}
                      />
                    </label>
                    <label>
                      Vòng
                      <input
                        type="number"
                        min={1}
                        max={100}
                        value={settings.numRounds}
                        onChange={(e) => patch("numRounds", Number(e.target.value) || 1)}
                      />
                    </label>
                  </div>
                  <div className="nurture-row">
                    <label>
                      Thích %
                      <input
                        type="number"
                        min={0}
                        max={100}
                        value={settings.likeProb}
                        onChange={(e) => patch("likeProb", Number(e.target.value) || 0)}
                      />
                    </label>
                    <label>
                      Bình luận %
                      <input
                        type="number"
                        min={0}
                        max={100}
                        value={settings.commentProb}
                        onChange={(e) => patch("commentProb", Number(e.target.value) || 0)}
                      />
                    </label>
                    <label>
                      Follow %
                      <input
                        type="number"
                        min={0}
                        max={100}
                        value={settings.followProb}
                        onChange={(e) => patch("followProb", Number(e.target.value) || 0)}
                      />
                    </label>
                    <label>
                      Vuốt nhanh %
                      <input
                        type="number"
                        min={0}
                        max={100}
                        value={settings.frenzyProb}
                        onChange={(e) => patch("frenzyProb", Number(e.target.value) || 0)}
                      />
                    </label>
                  </div>
                  <p className="hint">{actionHint}</p>
                  <div className="nurture-row">
                    <label>
                      Xem min (s)
                      <input
                        type="number"
                        step="0.5"
                        min={0.5}
                        max={120}
                        value={settings.watchMin}
                        onChange={(e) => patch("watchMin", Number(e.target.value) || 1)}
                      />
                    </label>
                    <label>
                      Xem max (s)
                      <input
                        type="number"
                        step="0.5"
                        min={0.5}
                        max={120}
                        value={settings.watchMax}
                        onChange={(e) => patch("watchMax", Number(e.target.value) || 5)}
                      />
                    </label>
                  </div>
                  <label>
                    Bundle TikTok
                    <input value={settings.bundleId} onChange={(e) => patch("bundleId", e.target.value)} />
                  </label>
                </div>
              )}

              <div className="nurture-sect nurture-sched">
                <label className="check">
                  <input
                    type="checkbox"
                    checked={settings.scheduleEnabled}
                    onChange={(e) => patch("scheduleEnabled", e.target.checked)}
                  />
                  Lịch tự chạy
                </label>
                <div className="nurture-row">
                  <label>
                    Mỗi (phút)
                    <input
                      type="number"
                      min={15}
                      max={1440}
                      value={settings.scheduleEveryMinutes}
                      onChange={(e) => patch("scheduleEveryMinutes", Number(e.target.value) || 240)}
                    />
                  </label>
                  <label>
                    Thời lượng (phút)
                    <input
                      type="number"
                      min={15}
                      max={360}
                      value={settings.scheduleDurationMinutes}
                      onChange={(e) => patch("scheduleDurationMinutes", Number(e.target.value) || 150)}
                    />
                  </label>
                </div>
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
