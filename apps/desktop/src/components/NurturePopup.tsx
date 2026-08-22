import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import { createPortal } from "react-dom";

import { InfoDot as Info } from "./InfoDot";
import {
  listenRiviuEvents,
  nurtureGetSettings,
  nurtureSaveSettings,
  nurtureSessionStatus,
  nurtureStart,
  nurtureStop,
  nurtureTestApi,
} from "../api";
import { exportViewJpegBurst } from "../viewStore";
import {
  BUDGET_TOTAL,
  budgetCeiling,
  budgetFree,
  budgetUsed,
  clampToBudget,
  fitToBudget,
  isOverBudget,
  isRateEnabled,
  type BudgetKey,
} from "../nurtureBudget";
import { targetsOf } from "./SelectionStrip";
import { IconApi, IconClose, IconHeart } from "./Icons";
import { LoadingState } from "./States";
import type {
  DeviceInfo,
  NurtureApiTestResult,
  NurtureSessionStatus,
  NurtureSettings,
} from "../types";
import { describeError } from "../describeError";

type Props = {
  devices: DeviceInfo[];
  selected: string[];
  onClose: () => void;
};

/**
 * A real switch rather than a bare checkbox.
 *
 * `appearance: none` on the input keeps it a checkbox to the accessibility tree and to
 * every test that finds it by label, so nothing about the semantics changes — only that it
 * reads as a control someone designed.
 */
function Switch({
  checked,
  onChange,
  label,
  what,
}: {
  checked: boolean;
  onChange: (value: boolean) => void;
  label: string;
  what: string;
}) {
  return (
    <label className="nu-switch">
      <input type="checkbox" checked={checked} onChange={(e) => onChange(e.target.checked)} />
      <span className="nu-switch-track" aria-hidden="true" />
      <span className="nu-switch-label">{label}</span>
      <Info of={label} what={what} />
    </label>
  );
}

/**
 * One feature: switch, name, and its percentage, on one aligned row.
 *
 * The switch is not a second way to write 0. Turning a feature off by zeroing its
 * percentage destroys the tuned number, so an operator pausing comments for one run has to
 * remember what 4 was. The switch stops the behaviour and keeps the number — which is what
 * the backend's `like_enabled`/`comment_enabled`/… fields are for. The number therefore
 * stays editable while the switch is off.
 */
function FeatureRow({
  label,
  what,
  percent,
  ceiling,
  enabled,
  onPercent,
  onEnabled,
}: {
  label: string;
  what: string;
  percent: number;
  /**
   * The highest this rate may reach: whatever the other three leave of the shared 100%.
   * Computed by `nurtureBudget`, never here.
   */
  ceiling: number;
  enabled: boolean;
  onPercent: (value: number) => void;
  onEnabled: (value: boolean) => void;
}) {
  return (
    <div className={`nu-feature nu-feature-ranged${enabled ? "" : " is-off"}`}>
      <label className="nu-switch nu-switch-bare">
        <input
          type="checkbox"
          checked={enabled}
          onChange={(e) => onEnabled(e.target.checked)}
          aria-label={`Bật ${label}`}
        />
        <span className="nu-switch-track" aria-hidden="true" />
      </label>
      <span className="nu-feature-name">
        {label}
        <Info of={label} what={what} />
      </span>
      {/* Every slider runs 0..100, always — the ceiling is enforced by `onPercent` clamping
          and *shown* as the pale part of the track, it is NOT the slider's `max`.
          Measured why: with `max` set to the ceiling, dragging row 2 up rescaled row 1,
          so row 1's thumb slid across the track while its number never changed (Follow at
          3 sat hard right on a max of 3, then jumped left when Thích freed 49 points). A
          thumb that moves on its own is a control lying about which row the operator is
          editing. A fixed scale also means 48% is the same distance along on all four rows,
          which is the only way four sliders read as shares of one thing.

          `--fill` / `--ceil` are fractions, turned into track positions in App.css. They are
          inset by half a thumb there, so the colour boundaries line up with the thumb centre
          instead of drifting up to 7px away from it at the ends. */}
      <input
        className="nu-feature-slider"
        type="range"
        min={0}
        max={BUDGET_TOTAL}
        step={1}
        value={percent}
        data-ceiling={ceiling}
        style={
          {
            "--fill": Math.min(percent, BUDGET_TOTAL) / BUDGET_TOTAL,
            "--ceil": Math.max(Math.min(percent, BUDGET_TOTAL), ceiling) / BUDGET_TOTAL,
          } as CSSProperties
        }
        title={
          enabled
            ? `Kéo được tới ${ceiling}% — ba tỉ lệ kia đang dùng ${BUDGET_TOTAL - ceiling}%`
            : `Đang tắt nên không tiêu ngân sách. Bật lại thì nó chiếm ${percent}%, mà hiện chỉ còn ${ceiling}% trống`
        }
        onChange={(e) => onPercent(Number(e.target.value) || 0)}
        aria-label={`${label} thanh kéo phần trăm`}
      />
      <label className="nu-feature-pct">
        <input
          type="number"
          min={0}
          // A switched-off row spends nothing, so the budget does not bound it — only 0..100
          // does. Same rule as `clampToBudget`, which is what actually holds either way.
          max={enabled ? ceiling : BUDGET_TOTAL}
          value={percent}
          onChange={(e) => onPercent(Number(e.target.value) || 0)}
          aria-label={`${label} phần trăm`}
        />
        <span aria-hidden="true">%</span>
      </label>
    </div>
  );
}

/** Marks a field a running session will not pick up, with the reason. */
function RestartBadge({ reason }: { reason: string }) {
  const [tip, setTip] = useState<{ left: number; top: number } | null>(null);
  const what = `${reason}. Đang chạy mà đổi thì phải bấm Dừng rồi Bắt đầu lại mới áp dụng.`;
  return (
    <span
      className="nurture-restart-badge"
      data-tip={what}
      onMouseEnter={(event) => {
        const rect = event.currentTarget.getBoundingClientRect();
        setTip({ left: Math.round(rect.left + rect.width / 2), top: Math.round(rect.top) });
      }}
      onMouseLeave={() => setTip(null)}
    >
      cần chạy lại
      {tip &&
        createPortal(
          <span className="nu-tip" role="tooltip" style={{ left: tip.left, top: tip.top }}>
            {what}
          </span>,
          document.body,
        )}
    </span>
  );
}

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
  // Tabs rather than a stack of collapsibles. The old panel put "Cấu hình AI", "Hành vi"
  // and the schedule one under another in a column narrow enough that each of them had to
  // be folded away, so tuning two related numbers meant scrolling past a closed section —
  // and opening two at once pushed the live log off the bottom, which is the one thing the
  // panel is open to watch. One group at a time, full width, log pinned above.
  const [tab, setTab] = useState<"behaviour" | "ai" | "schedule">("behaviour");
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
      setMsg(describeError(e));
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

  /// One of the four rates that share the 100% budget.
  ///
  /// Clamped against the *current* settings inside the updater rather than against a
  /// captured copy: a slider fires many times a second while dragging, and a ceiling
  /// computed from a stale render lets the sum drift past the budget between two frames.
  const patchRate = (key: BudgetKey, value: number) => {
    setSettings((prev) => (prev ? { ...prev, [key]: clampToBudget(prev, key, value) } : prev));
  };

  /// A saved config can spend more than the budget, because nothing added the four rates
  /// up before today. The panel says so and offers the fix rather than editing it silently.
  const overBudget = settings ? isOverBudget(settings) : false;

  const save = async (next?: NurtureSettings): Promise<boolean> => {
    const s = next ?? settings;
    if (!s) return false;
    // The four rates share one budget, so the check is over all four. It replaces a pair
    // of narrower ones ("Thích + Bình luận > 100" and "Follow/vuốt nhanh phải 0..100")
    // that could both pass while the four together spent 131.
    if (isOverBudget(s)) {
      setMsg(
        `Bốn tỉ lệ tương tác dùng chung ${BUDGET_TOTAL}%, đang là ${budgetUsed(s)}% — kéo xuống cho vừa`,
      );
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
    // The switch, not just the number: a percentage kept for later while the switch is off
    // cannot produce a comment — `NurtureSettings::into_effective` zeroes it before the loop
    // ever sees it — so demanding an API key for it was refusing a save over a feature that
    // provably will not run.
    if (isRateEnabled(s, "commentProb") && s.commentProb > 0 && !s.apiKey.trim()) {
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
      setMsg(describeError(e));
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
      setMsg(describeError(e));
    } finally {
      setBusy(false);
    }
  };

  const runApiTest = async () => {
    if (!settings) return;
    const udid = testUdid || fallbackTestUdid;
    if (!udid) {
      setMsg("Chọn một máy trước khi test API");
      return;
    }
    setApiTesting(true);
    setApiTest(null);
    setMsg(null);
    try {
      if (!(await save(settings))) return;
      setApiTest(await nurtureTestApi(udid, await exportViewJpegBurst(udid)));
    } catch (e) {
      setMsg(describeError(e));
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
                      setMsg(describeError(e));
                    } finally {
                      setBusy(false);
                    }
                  }}
                >
                  Lưu
                </button>
              </div>

              {statuses.length > 0 && (
                <div className="nurture-live">
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
                          <div className="grow" />
                          {/* The same four numbers as before, but as labelled cells: the old
                              single string ("12/34v · ♥5/6 · BL1/1 · +0/0") packed done-vs-
                              attempted for four different things into one line, and the only
                              way to read it was the tooltip. The tooltip stays. */}
                          <span
                            className="nurture-metrics"
                            title="đã xác nhận / đã thử — video · tim · bình luận · follow"
                          >
                            <b>{s.videosDone}</b>
                            <i>/{s.swipeAttempts}</i>
                            <em>v</em>
                            <b>{s.likes}</b>
                            <i>/{s.likeAttempts}</i>
                            <em>♥</em>
                            <b>{s.comments}</b>
                            <i>/{s.commentAttempts}</i>
                            <em>BL</em>
                            <b>{s.follows}</b>
                            <i>/{s.followAttempts}</i>
                            <em>+</em>
                          </span>
                        </div>
                        <p className="nurture-float-log-msg">{statusVi(s.lastMessage)}</p>
                      </div>
                    ))}
                  </div>
                </div>
              )}
              {msg && <p className="nurture-float-err">{msg}</p>}

              <div className="nurture-tabs" role="tablist" aria-label="Cấu hình">
                {(
                  [
                    ["behaviour", "Hành vi"],
                    ["ai", "AI"],
                    ["schedule", "Lịch"],
                  ] as const
                ).map(([key, label]) => (
                  <button
                    key={key}
                    type="button"
                    role="tab"
                    aria-selected={tab === key}
                    className={`nurture-tab${tab === key ? " is-on" : ""}`}
                    onClick={() => setTab(key)}
                  >
                    {label}
                  </button>
                ))}
                <div className="grow" />
                {anyRunning && (
                  <span className="nurture-live-flag" title="Bấm Lưu là áp ngay từ bài kế tiếp">
                    đang chạy · Lưu để áp ngay
                  </span>
                )}
              </div>

              {tab === "ai" && (
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
              )}

              {tab === "behaviour" && (
                <div className="nurture-sect nu-pane" role="tabpanel">
                  <div className="nu-grid">
                    <label className="nu-field">
                      <span className="nu-label">
                        Giới hạn video
                        <Info
                          of="Giới hạn video"
                          what="Phiên dừng sau đúng số video này (nhân với số vòng). Thời lượng phiên vẫn là trần riêng: cái nào tới trước thì dừng."
                        />
                        <RestartBadge reason="Mục tiêu của phiên được tính lúc bắt đầu" />
                      </span>
                      <input
                        type="number"
                        min={1}
                        max={10000}
                        value={settings.numVideos}
                        onChange={(e) => patch("numVideos", Number(e.target.value) || 1)}
                      />
                    </label>
                    <label className="nu-field">
                      <span className="nu-label">
                        Vòng
                        <Info
                          of="Vòng"
                          what="Nhân với giới hạn video để ra tổng số video của phiên: 15 video × 2 vòng = 30 video."
                        />
                        <RestartBadge reason="Mục tiêu của phiên được tính lúc bắt đầu" />
                      </span>
                      <input
                        type="number"
                        min={1}
                        max={100}
                        value={settings.numRounds}
                        onChange={(e) => patch("numRounds", Number(e.target.value) || 1)}
                      />
                    </label>
                  </div>

                  <div className="nu-group">
                    <div className="nu-group-head">
                      Tương tác
                      {/* The budget, stated where it is spent: four rates sharing a hundred
                          need the remainder on screen or every drag is a guess. */}
                      <span className={`nu-budget${overBudget ? " is-over" : ""}`}>
                        {overBudget
                          ? `Đang dùng ${budgetUsed(settings)}% / ${BUDGET_TOTAL}%`
                          : `Còn ${budgetFree(settings)}% / ${BUDGET_TOTAL}%`}
                      </span>
                    </div>
                    {overBudget && (
                      /* Two ways to get here: a config saved before the budget existed, and a
                         switch turned back on over a number that no longer fits. Both leave
                         every ceiling at or below where its rate already is, so no slider can
                         be dragged up. Said out loud with a one-press fix rather than rewritten
                         on load or on the switch click: these are the operator's tuned numbers,
                         and something silently editing them is worse than a sentence asking. */
                      <p className="nu-budget-warn" role="alert">
                        Các tỉ lệ đang bật dùng chung {BUDGET_TOTAL}%, mà cộng lại đang là{" "}
                        {budgetUsed(settings)}%. Kéo xuống cho vừa, hoặc{" "}
                        <button
                          type="button"
                          className="link"
                          onClick={() =>
                            setSettings((prev) => (prev ? { ...prev, ...fitToBudget(prev) } : prev))
                          }
                        >
                          đưa về {BUDGET_TOTAL}%
                        </button>{" "}
                        (trừ dần từ tỉ lệ lớn nhất).
                      </p>
                    )}
                    <FeatureRow
                      label="Thích"
                      what="Tỉ lệ post được thả tim. Chỉ tính thành công khi nhãn nút tim đổi trạng thái, không phải khi tap xong — nên số 'đã tim' luôn nhỏ hơn hoặc bằng số lần thử."
                      percent={settings.likeProb}
                      ceiling={budgetCeiling(settings, "likeProb")}
                      enabled={settings.likeEnabled ?? true}
                      onPercent={(v) => patchRate("likeProb", v)}
                      onEnabled={(v) => patch("likeEnabled", v)}
                    />
                    <FeatureRow
                      label="Bình luận"
                      what="Tỉ lệ post được bình luận. AI đọc nội dung post rồi tự viết; chỉ tính là đã gửi khi nút Gửi tắt lại. Cần API key ở tab AI."
                      percent={settings.commentProb}
                      ceiling={budgetCeiling(settings, "commentProb")}
                      enabled={settings.commentEnabled ?? true}
                      onPercent={(v) => patchRate("commentProb", v)}
                      onEnabled={(v) => patch("commentEnabled", v)}
                    />
                    <FeatureRow
                      label="Follow"
                      what="Tỉ lệ post được follow tác giả, tính riêng chứ không kèm thích hay bình luận. Xác nhận bằng việc nút Follow mất khỏi thẻ."
                      percent={settings.followProb}
                      ceiling={budgetCeiling(settings, "followProb")}
                      enabled={settings.followEnabled ?? true}
                      onPercent={(v) => patchRate("followProb", v)}
                      onEnabled={(v) => patch("followEnabled", v)}
                    />
                    <FeatureRow
                      label="Vuốt nhanh"
                      what="Tỉ lệ post bị vuốt qua nhanh, không xem hết — giống lúc người ta lướt cho qua mấy bài không quan tâm."
                      percent={settings.frenzyProb}
                      ceiling={budgetCeiling(settings, "frenzyProb")}
                      enabled={settings.frenzyEnabled ?? true}
                      onPercent={(v) => patchRate("frenzyProb", v)}
                      onEnabled={(v) => patch("frenzyEnabled", v)}
                    />
                  </div>

                  <div className="nu-group">
                    <div className="nu-group-head">Nhịp</div>
                    <div className="nu-grid">
                      <label className="nu-field">
                        <span className="nu-label">
                          Xem min
                          <Info
                            of="Xem min"
                            what="Số giây ít nhất dừng lại ở mỗi post. Nhịp phiên còn nhân thêm hệ số theo tâm trạng, nên số 'xem' trong log có thể ra ngoài khoảng min–max."
                          />
                        </span>
                        <input
                          type="number"
                          step="0.5"
                          min={0.5}
                          max={120}
                          value={settings.watchMin}
                          onChange={(e) => patch("watchMin", Number(e.target.value) || 1)}
                        />
                      </label>
                      <label className="nu-field">
                        <span className="nu-label">
                          Xem max
                          <Info
                            of="Xem max"
                            what="Số giây nhiều nhất dừng lại ở mỗi post, trước khi nhân hệ số nhịp. Đặt sát min thì phiên đều đặn hơn nhưng cũng máy móc hơn."
                          />
                        </span>
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
                    <Switch
                      checked={settings.fatigue}
                      onChange={(v) => patch("fatigue", v)}
                      label="Mỏi dần"
                      what="Càng về cuối phiên càng xem lâu và tương tác thưa hơn, thay vì giữ đúng một nhịp từ đầu tới cuối. Bật thì số tim thực tế thấp hơn tỉ lệ đã đặt."
                    />
                    <Switch
                      checked={settings.timeOfDay}
                      onChange={(v) => patch("timeOfDay", v)}
                      label="Theo giờ trong ngày"
                      what="Nhịp thay đổi theo giờ thật của máy tính: đêm và giờ làm thì chậm và ít tương tác hơn giờ cao điểm."
                    />
                    <Switch
                      checked={settings.pauseSwipe}
                      onChange={(v) => patch("pauseSwipe", v)}
                      label="Ngập ngừng khi vuốt"
                      what="Thỉnh thoảng vuốt nửa vời rồi mới vuốt hẩn, và thời gian mỗi cú vuốt không đều nhau."
                    />
                    <Switch
                      checked={settings.humanLimits ?? false}
                      onChange={(v) => patch("humanLimits", v)}
                      label="Giới hạn nhịp người"
                      what="Tắt (mặc định): các tỉ lệ bạn đặt ở trên là tỉ lệ thực. Bật: engine tự áp thêm trần 8–16 tim / 1–3 bình luản / 1–2 follow mỗi giờ, chỉ cho tương tác 2 trong 5 bài gần nhất, chờ 12–35 giây sau mỗi hành động và nghỉ 15–90 giây mỗi 7–13 bài — phiên trông giống người hơn nhưng chạy ít hơn nhiều so với số bạn đặt."
                    />
                    <div className="nu-grid">
                      <label className="nu-field">
                        <span className="nu-label">
                          Nghỉ đêm từ
                          <Info
                            of="Nghỉ đêm"
                            what="Rơi vào khoảng giờ này thì phiên tự dừng, tính theo giờ máy tính. Để 0 và 0 là không nghỉ đêm."
                          />
                        </span>
                        <input
                          type="number"
                          min={0}
                          max={23}
                          value={settings.nightStart}
                          onChange={(e) => patch("nightStart", Number(e.target.value) || 0)}
                        />
                      </label>
                      <label className="nu-field">
                        <span className="nu-label">đến</span>
                        <input
                          type="number"
                          min={0}
                          max={23}
                          value={settings.nightEnd}
                          onChange={(e) => patch("nightEnd", Number(e.target.value) || 0)}
                        />
                      </label>
                    </div>
                  </div>

                  <div className="nu-group">
                    <div className="nu-group-head">Bài ảnh</div>
                    <div className="nu-feature">
                      <label className="nu-switch nu-switch-bare">
                        <input
                          type="checkbox"
                          checked={settings.carouselEnabled ?? true}
                          onChange={(e) => patch("carouselEnabled", e.target.checked)}
                          aria-label="Bật vuốt ngang bài ảnh"
                        />
                        <span className="nu-switch-track" aria-hidden="true" />
                      </label>
                      <span className="nu-feature-name">
                        Vuốt ngang
                        <Info
                          of="Vuốt ngang"
                          what="Bài nhiều ảnh thì vuốt ngang xem tiếp, thay vì bỏ qua sau ảnh đầu. Phần trăm bên cạnh là xem bao nhiêu phần của bài."
                        />
                      </span>
                      <label className="nu-feature-pct">
                        <input
                          type="number"
                          min={1}
                          max={100}
                          step={5}
                          value={settings.carouselPortionPercent ?? 100}
                          onChange={(e) =>
                            patch("carouselPortionPercent", Number(e.target.value) || 100)
                          }
                          aria-label="Xem bao nhiêu phần trăm bài ảnh"
                          title="100% là xem tới hết bài — dừng khi một cú vuốt không còn làm đổi ảnh. 50% là xem khoảng nửa bài rồi vuốt sang bài khác."
                        />
                        <span aria-hidden="true">%</span>
                      </label>
                    </div>
                  </div>

                  <label className="nu-field">
                    <span className="nu-label">
                      Bundle TikTok
                      <Info
                        of="Bundle TikTok"
                        what="App id của TikTok. Trên Android app tự tìm package đã cài trên từng máy nên thường không cần sửa ô này; nó chủ yếu dành cho iPhone."
                      />
                      <RestartBadge reason="App đã mở rồi; trên Android package được resolve theo từng máy" />
                    </span>
                    <input value={settings.bundleId} onChange={(e) => patch("bundleId", e.target.value)} />
                  </label>
                </div>
              )}

              {tab === "schedule" && (
              <div className="nurture-sect nurture-sched" role="tabpanel">
                <label className="check">
                  <input
                    type="checkbox"
                    checked={settings.scheduleEnabled}
                    onChange={(e) => patch("scheduleEnabled", e.target.checked)}
                  />
                  <span className="nu-inline">
                    Lịch tự chạy
                    <Info
                      of="Lịch tự chạy"
                      what="Tự khởi động phiên theo chu kỳ, không cần bấm Bắt đầu. Chỉ chạy trên những máy đã chọn khi lưu."
                    />
                  </span>
                </label>
                <div className="nurture-row">
                  <label>
                    <span className="nu-inline">
                      Mỗi (phút)
                      <Info of="Mỗi (phút)" what="Khoảng cách giữa hai lần tự khởi động." />
                    </span>
                    <input
                      type="number"
                      min={15}
                      max={1440}
                      value={settings.scheduleEveryMinutes}
                      onChange={(e) => patch("scheduleEveryMinutes", Number(e.target.value) || 240)}
                    />
                  </label>
                  <label>
                    <span className="nu-inline">
                      Thời lượng (phút)
                      <Info
                        of="Thời lượng (phút)"
                        what="Phiên theo lịch chạy tối đa bấy nhiêu phút. Phiên bấm tay không dùng số này — nó được gán một trần 2–3 giờ ngẫu nhiên, nên hai máy bấm cùng lúc không dừng cùng lúc."
                      />
                    </span>
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
              )}
            </>
          )}
        </div>
      </div>
    </div>
  );
}
