import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties } from "react";

import { InfoDot as Info } from "./InfoDot";
import {
  listenRiviuEvents,
  nurtureGetSettings,
  nurtureSaveSettings,
  nurtureSessionLogSummary,
  nurtureSessionStatus,
  nurtureStart,
  nurtureStop,
} from "../api";
import {
  BUDGET_TOTAL,
  budgetUsed,
  clampToBudget,
  isOverBudget,
  isRateEnabled,
  type BudgetKey,
} from "../nurtureBudget";
import { targetsOf } from "../selectionTargets";
import { orderDevicesByNumber, tileName, tileNumber } from "../deviceNaming";
import { useTickWhile } from "../useTickWhile";
import { NurtureAiTab } from "./nurture/NurtureAiTab";
import { NurtureCommentsTab } from "./nurture/NurtureCommentsTab";
import { NurtureDeviceLog } from "./nurture/NurtureDeviceLog";
import { NurtureDeviceProgress, NurtureRunProgress } from "./nurture/NurtureProgress";
import { NurtureBehaviourTab } from "./nurture/NurtureBehaviourTab";
import { IconClose, IconHeart } from "./Icons";
import { LoadingState } from "./States";
import type {
  DeviceInfo,
  DeviceMeta,
  NurtureSessionStatus,
  NurtureSettings,
  SessionLogSummary,
} from "../types";
import { describeError } from "../describeError";

/**
 * One line in the live list.
 *
 * `status` is null for a phone that has history but never ran a session — the idle popup
 * sweep leaves those. Keeping them in the same list rather than a second section is
 * deliberate: from the operator's side "what has this phone been doing" is one question,
 * and splitting the answer by which subsystem happened to write it would be an
 * implementation detail leaking into the panel.
 */
type NurtureRow = {
  udid: string;
  running: boolean;
  message: string;
  status: NurtureSessionStatus | null;
};

type Props = {
  devices: DeviceInfo[];
  selected: string[];
  metas: Map<string, DeviceMeta>;
  onClose: () => void;
};

/**
 * A real switch rather than a bare checkbox.
 *
 * `appearance: none` on the input keeps it a checkbox to the accessibility tree and to
 * every test that finds it by label, so nothing about the semantics changes — only that it
 * reads as a control someone designed.
 */
export function Switch({
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
export function FeatureRow({
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

/** Map engine status English → short Vietnamese for the live log. */
function statusVi(raw: string): string {
  const s = raw.trim();
  if (!s) return "—";
  const exact: Record<string, string> = {
    starting: "Đang khởi động…",
    queued: "Đang xếp hàng…",
    stopped: "Đã dừng",
    done: "Xong phiên",
    "ui session": "Mở phiên điều khiển…",
    "launch TikTok": "Mở TikTok…",
    "ui session: timeout": "Phiên điều khiển quá hạn — thử lại",
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
  // Not "WDA": that is the iOS agent, and thirteen of the fourteen phones on this
  // desk are Android. The status stream is shared by both platforms, so the word has
  // to be one that is true of either.
  if (s.startsWith("ui session:"))
    return `Phiên điều khiển: ${s.slice("ui session:".length).trim()}`;
  if (s.startsWith("error:")) return `Lỗi: ${s.slice("error:".length).trim()}`;
  return s;
}

function deviceLabel(
  devices: DeviceInfo[],
  metas: Map<string, DeviceMeta>,
  udid: string,
): string {
  const d = devices.find((x) => x.udid === udid);
  const meta = metas.get(udid);
  if (!d) {
    return `Máy ${meta?.number ?? "?"} · ${meta?.alias?.trim() || udid.slice(0, 8)}`;
  }
  const ordered = orderDevicesByNumber(devices, metas);
  const position = ordered.findIndex((device) => device.udid === udid) + 1;
  return `Máy ${tileNumber(position || 1, meta)} · ${tileName(d, meta)}`;
}

export function NurturePopup({ devices, selected, metas, onClose }: Props) {
  const [settings, setSettings] = useState<NurtureSettings | null>(null);
  const [statuses, setStatuses] = useState<NurtureSessionStatus[]>([]);
  const [msg, setMsg] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // Tabs rather than a stack of collapsibles. The old panel put "Cấu hình AI", "Hành vi"
  // and the schedule one under another in a column narrow enough that each of them had to
  // be folded away, so tuning two related numbers meant scrolling past a closed section —
  // and opening two at once pushed the live log off the bottom, which is the one thing the
  // panel is open to watch. One group at a time, full width, with the log in the same tab row.
  const [tab, setTab] = useState<"behaviour" | "ai" | "comments" | "log">("behaviour");
  /**
   * Which device's history is open, or `null`.
   *
   * One at a time, not a set. Two open logs in a panel this narrow means neither is
   * readable, and the question being asked is always about one phone — the row that says
   * something surprising.
   */
  const [openLog, setOpenLog] = useState<string | null>(null);
  /**
   * Phones that have said something, whether or not they ever ran a session.
   *
   * The rows used to come from the live statuses alone, and the idle sweep produces
   * neither — so a phone it had just unstuck off TikTok's onboarding page had a full
   * history and no row anywhere to open it from. That is the whole point of the sweep
   * writing into the same book, so the rows have to come from both.
   */
  const [logged, setLogged] = useState<SessionLogSummary[]>([]);
  const [pos, setPos] = useState({ x: 0, y: 0 });
  const drag = useRef<{ ox: number; oy: number; sx: number; sy: number } | null>(null);
  const targets = targetsOf(selected, devices);
  const anyRunning = statuses.some((s) => s.running);

  const totals = useMemo(() => {
    return statuses.reduce(
      (acc, s) => {
        acc.videos += s.videosDone;
        acc.likes += s.likes;
        acc.comments += s.comments;
        acc.follows += s.follows;
        // Tokens, not money. The USD that used to sit here was two hand-typed per-million
        // prices multiplied by exactly these counts, and no form could edit them — so after
        // any model change every figure was silently wrong. These come from the API's own
        // `usage`, so they are true of whatever model is configured.
        acc.promptTokens += s.sessionPromptTokens;
        acc.completionTokens += s.sessionCompletionTokens;
        return acc;
      },
      { videos: 0, likes: 0, comments: 0, follows: 0, promptTokens: 0, completionTokens: 0 },
    );
  }, [statuses]);

  /**
   * One row per phone with anything to show: a live status if it has one, otherwise the
   * last line the idle sweep left. Running phones first — they are what the panel is open
   * to watch — then the rest by udid so the list does not shuffle under the cursor.
   */
  /// One of the two bounds on a session is a wall clock, so the bars have to advance between
  /// status pushes — a phone watching a long video emits nothing for twenty seconds. Ticking
  /// only while something runs keeps a panel left open on a finished run quiet.
  const nowTick = useTickWhile(statuses.some((s) => s.running));

  const rows = useMemo((): NurtureRow[] => {
    const withStatus = new Set(statuses.map((s) => s.udid));
    const fromStatus: NurtureRow[] = statuses.map((status) => ({
      udid: status.udid,
      running: status.running,
      message: status.lastMessage,
      status,
    }));
    const logOnly: NurtureRow[] = logged
      .filter((entry) => !withStatus.has(entry.udid))
      .map((entry) => ({
        udid: entry.udid,
        running: false,
        message: entry.last?.text ?? "",
        status: null,
      }));
    // Running first — that is what the panel is open to watch — then **failures**, then the
    // rest. Failures used to sort to the bottom with the finished runs and render as the same
    // grey row, which is how two dead phones went unnoticed on a fourteen-phone run.
    const rank = (r: NurtureRow) =>
      r.running ? 0 : r.status?.outcome === "failed" ? 1 : 2;
    return [...fromStatus, ...logOnly].sort(
      (a, b) => rank(a) - rank(b) || a.udid.localeCompare(b.udid),
    );
  }, [statuses, logged]);

  const reload = useCallback(async () => {
    try {
      const [s, st, summary] = await Promise.all([
        nurtureGetSettings(),
        nurtureSessionStatus(),
        nurtureSessionLogSummary(),
      ]);
      setSettings(s);
      setStatuses(st);
      setLogged(summary);
      setMsg(null);
    } catch (e) {
      setMsg(describeError(e));
    }
  }, []);

  useEffect(() => {
    reload();
  }, [reload]);

  /**
   * The idle sweep writes lines without emitting a status, so its rows can only appear by
   * asking. Five seconds against a sweep every forty-five: slow enough to be free, quick
   * enough that a phone unstuck while the panel is open shows up in it.
   */
  useEffect(() => {
    const timer = setInterval(() => {
      nurtureSessionLogSummary()
        .then(setLogged)
        .catch(() => undefined);
    }, 5_000);
    return () => clearInterval(timer);
  }, []);

  useEffect(() => {
    let un: (() => void) | undefined;
    listenRiviuEvents((event) => {
      if (event.type !== "nurtureStatus") return;
      const st = event.status;
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
      setTab("log");
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

              <div className="nurture-tabs" role="tablist" aria-label="Nuôi TikTok">
                {(
                  [
                    ["behaviour", "Hành vi"],
                    ["ai", "AI"],
                    ["comments", "Bình luận"],
                    ["log", "Log"],
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
              </div>

              {tab === "log" && (
                <div className="nurture-live" role="tabpanel">
                  {rows.length > 0 ? (
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
                    {/* Rendered at all, which is the point: the AI spend was recorded for
                        months and shown nowhere, so nobody could see that the number was
                        fabricated. Only appears once a comment has actually cost something. */}
                    {totals.promptTokens + totals.completionTokens > 0 && (
                      <div title="token vào / ra mà API báo — nhân với giá thật của provider để ra tiền">
                        <span>Token AI</span>
                        <strong>
                          {totals.promptTokens.toLocaleString("vi-VN")}/
                          {totals.completionTokens.toLocaleString("vi-VN")}
                        </strong>
                      </div>
                    )}
                  </div>
                  {/* Above the per-device rows, because it is the answer to the question the
                      operator asks first. Renders nothing until a row carries a run id, so a
                      panel showing only idle-sweep rows is unchanged. */}
                  <NurtureRunProgress statuses={statuses} now={nowTick} />
                  <div
                    className={`nurture-float-log${openLog ? " is-expanded" : ""}`}
                    aria-live="polite"
                  >
                    {rows.map((row) => (
                      <div
                        key={row.udid}
                        className={`nurture-float-log-row${row.running ? " is-run" : ""}${
                          row.status?.outcome === "failed" ? " is-failed" : ""
                        }${
                          row.status?.outcome && row.status.outcome !== "failed" ? " is-done" : ""
                        }${openLog === row.udid ? " is-open" : ""}`}
                      >
                        {/* The head is the control. A row is the only handle the operator has
                            on one phone here, and `lastMessage` — one overwritten string — was
                            the whole of what it could say. Opening it asks the Rust ring for
                            the rest. A real `button` rather than an `onClick` div, so it is
                            reachable by keyboard and announces its own state. */}
                        <button
                          type="button"
                          className="nurture-float-log-head"
                          aria-expanded={openLog === row.udid}
                          title={openLog === row.udid ? "Đóng nhật ký" : "Xem nhật ký riêng máy này"}
                          onClick={() => setOpenLog((prev) => (prev === row.udid ? null : row.udid))}
                        >
                          <span
                            className={`nurture-dot${row.running ? " on" : ""}${
                              row.status?.outcome === "failed" ? " bad" : ""
                            }`}
                          />
                          <strong title={row.udid}>{deviceLabel(devices, metas, row.udid)}</strong>
                          <div className="grow" />
                          {/* The same four numbers as before, but as labelled cells: the old
                              single string ("12/34v · ♥5/6 · BL1/1 · +0/0") packed done-vs-
                              attempted for four different things into one line, and the only
                              way to read it was the tooltip. The tooltip stays. */}
                          {/* Only a phone that ran a session has counters. A row that
                              exists because the idle sweep unstuck it has none, and printing
                              "0/0v ♥0/0" against it would read as a session that did
                              nothing rather than as no session at all. */}
                          {row.status ? (
                            <span
                              className="nurture-metrics"
                              title="đã xác nhận / đã thử — video · tim · bình luận · follow"
                            >
                              <b>{row.status.videosDone}</b>
                              <i>/{row.status.swipeAttempts}</i>
                              <em>v</em>
                              <b>{row.status.likes}</b>
                              <i>/{row.status.likeAttempts}</i>
                              <em>♥</em>
                              <b>{row.status.comments}</b>
                              <i>/{row.status.commentAttempts}</i>
                              <em>BL</em>
                              <b>{row.status.follows}</b>
                              <i>/{row.status.followAttempts}</i>
                              <em>+</em>
                            </span>
                          ) : (
                            <span className="nurture-metrics is-idle" title="chưa chạy phiên nào — dòng này do bộ tự khôi phục popup ghi">
                              tự khôi phục
                            </span>
                          )}
                          <span className="nurture-log-chevron" aria-hidden="true">
                            {openLog === row.udid ? "▾" : "▸"}
                          </span>
                        </button>
                        <p className="nurture-float-log-msg">{statusVi(row.message)}</p>
                        {/* Outside the head `<button>` on purpose: a `progressbar` nested in a
                            button is neither, and the row head has to stay a plain control.
                            Gated on `row.status` so an idle-sweep row — which never ran a
                            session and has no target — does not draw a bar stuck at 0%. */}
                        {row.status && (
                          <NurtureDeviceProgress status={row.status} now={nowTick} />
                        )}
                        {openLog === row.udid && (
                          <NurtureDeviceLog udid={row.udid} running={row.running} />
                        )}
                      </div>
                    ))}
                  </div>
                    </>
                  ) : (
                    <p className="nurture-log-empty">Chưa có log nuôi TikTok.</p>
                  )}
                </div>
              )}
              {msg && <p className="nurture-float-err">{msg}</p>}

              {tab === "ai" && (
                <NurtureAiTab
                  settings={settings}
                  patch={patch}
                  devices={devices}
                  targets={targets}
                  save={save}
                  onMessage={setMsg}
                />
              )}
              {tab === "behaviour" && (
                <NurtureBehaviourTab
                  settings={settings}
                  patch={patch}
                  patchRate={patchRate}
                  setSettings={setSettings}
                  overBudget={overBudget}
                  targets={targets}
                />
              )}

              {tab === "comments" && <NurtureCommentsTab live={anyRunning} />}
            </>
          )}
        </div>
      </div>
    </div>
  );
}
