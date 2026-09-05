import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";

import { InfoDot as Info } from "./InfoDot";
import { AutomationProfileControl } from "./AutomationProfileControl";
import {
  listenRiviuEvents,
  nurtureGetSettings,
  nurtureSaveSettings,
  nurtureSessionLogSummary,
  nurtureSessionStatus,
  nurtureStart,
  nurtureStop,
} from "../api";
import { targetsOf } from "../selectionTargets";
import { nurtureProfileConfig } from "../automationProfileConfig";
import { orderDevicesByNumber, tileName, tileNumber } from "../deviceNaming";
import { useTickWhile } from "../useTickWhile";
import { NurtureAiTab } from "./nurture/NurtureAiTab";
import { NurtureCommentsTab } from "./nurture/NurtureCommentsTab";
import { NurtureDeviceLog } from "./nurture/NurtureDeviceLog";
import { NurtureDeviceProgress, NurtureRunProgress } from "./nurture/NurtureProgress";
import { NurtureBehaviourTab } from "./nurture/NurtureBehaviourTab";
import { IconClose, IconHeart, IconRefresh } from "./Icons";
import { LoadingState, StatusNotice } from "./States";
import { CommandBar, StatusChip, SummaryRail } from "./WorkspacePrimitives";
import type {
  DeviceInfo,
  DeviceMeta,
  NurtureSessionStatus,
  NurtureSettings,
  SessionLogSummary,
  TargetRef,
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
  /** Already resolved from All/Group/Explicit at this render; an empty array means no target. */
  targetUdids?: string[];
  targetRef?: TargetRef;
  metas: Map<string, DeviceMeta>;
  onClose?: () => void;
  surface?: "popup" | "page";
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
  enabled,
  onPercent,
  onEnabled,
}: {
  label: string;
  what: string;
  percent: number;
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
      {/* Every action is its own 0..100 roll. */}
      <input
        className="nu-feature-slider"
        type="range"
        min={0}
        max={100}
        step={1}
        value={percent}
        data-ceiling={100}
        style={
          {
            "--fill": Math.min(percent, 100) / 100,
            "--ceil": 1,
          } as CSSProperties
        }
        title={`${label}: ${percent}%`}
        onChange={(e) => onPercent(Number(e.target.value) || 0)}
        aria-label={`${label} thanh kéo phần trăm`}
      />
      <label className="nu-feature-pct">
        <input
          type="number"
          min={0}
          max={100}
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
    save: "Đang lưu",
    follow: "Đang theo dõi tác giả",
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
  const saveReasonVi = (reason: string): string => {
    const normalized = reason.trim();
    const reasons: Record<string, string> = {
      "state unreadable": "không đọc được trạng thái",
      "audit unavailable": "không ghi được nhật ký",
      "card changed": "thẻ đã đổi",
      "no control": "không tìm thấy nút Lưu",
    };
    return reasons[normalized] ?? normalized;
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
  if (s.startsWith("save skip:")) return `Bỏ lưu: ${saveReasonVi(s.slice("save skip:".length))}`;
  if (s.startsWith("save fail:")) return `Lưu lỗi: ${saveReasonVi(s.slice("save fail:".length))}`;
  if (s.startsWith("save uncertain:"))
    return `Lưu chưa chắc chắn: ${saveReasonVi(s.slice("save uncertain:".length))}`;
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
    return `Máy ${meta?.number ?? "?"} · ${meta?.alias?.trim() || "đã rời danh sách"}`;
  }
  const ordered = orderDevicesByNumber(devices, metas);
  const position = ordered.findIndex((device) => device.udid === udid) + 1;
  return `Máy ${tileNumber(position || 1, meta)} · ${tileName(d, meta)}`;
}

function CleanupStatus({ status }: { status: NurtureSessionStatus }) {
  const state = status.cleanupState ?? "pending";
  if (state === "processAbsent" && status.cleanupProof) {
    return (
      <div className="nurture-cleanup-status">
        <StatusChip tone="success">TikTok đã tắt</StatusChip>
        <details>
          <summary>Chứng cứ tiến trình</summary>
          <dl>
            <div><dt>Gói ứng dụng</dt><dd>{status.cleanupProof.bundleId}</dd></div>
            <div><dt>PID trước khi tắt</dt><dd>{status.cleanupProof.oldPid ?? "Không chạy"}</dd></div>
          </dl>
          {status.cleanupError && <p role="alert">{status.cleanupError}</p>}
        </details>
      </div>
    );
  }
  if (state === "failed") {
    return (
      <div className="nurture-cleanup-status">
        <StatusChip tone="error">Chưa xác nhận được TikTok đã tắt</StatusChip>
        {status.cleanupError && (
          <details>
            <summary>Chi tiết lỗi</summary>
            <p role="alert">{status.cleanupError}</p>
          </details>
        )}
      </div>
    );
  }
  return (
    <StatusChip tone={status.running ? "info" : "warning"}>
      {status.running ? "Dọn ứng dụng khi kết thúc" : "Chưa có chứng cứ tắt ứng dụng"}
    </StatusChip>
  );
}

export function NurturePopup({
  devices,
  selected,
  targetUdids,
  targetRef,
  metas,
  onClose,
  surface = "popup",
}: Props) {
  const [settings, setSettings] = useState<NurtureSettings | null>(null);
  const [statuses, setStatuses] = useState<NurtureSessionStatus[]>([]);
  const [startedTargets, setStartedTargets] = useState<string[]>([]);
  const [msg, setMsg] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // Tabs rather than a stack of collapsibles. The old panel put "Cấu hình AI", "Hành vi"
  // and the schedule one under another in a column narrow enough that each of them had to
  // be folded away, so tuning two related numbers meant scrolling past a closed section —
  // and opening two at once pushed the live log off the bottom, which is the one thing the
  // panel is open to watch. One group at a time, full width, with the log in the same tab row.
  const [tab, setTab] = useState<"behaviour" | "ai" | "comments" | "log">("behaviour");
  const [pageMode, setPageMode] = useState<"setup" | "monitor">("setup");
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
  const targets = targetUdids ?? targetsOf(selected, devices);
  const anyRunning = statuses.some((s) => s.running);
  const runningTargets = useMemo(
    () => [...new Set(statuses.filter((status) => status.running).map((status) => status.udid))],
    [statuses],
  );
  const stopTargets = startedTargets.length > 0 ? startedTargets : runningTargets;
  const pageSurface = surface === "page";
  const profileConfig = useMemo(
    () =>
      settings ? nurtureProfileConfig(settings, settings.scheduleDurationMinutes) : null,
    [settings],
  );

  const totals = useMemo(() => {
    return statuses.reduce(
      (acc, s) => {
        acc.videos += s.videosDone;
        acc.likes += s.likes;
        acc.saves += s.saves ?? 0;
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
      {
        videos: 0,
        likes: 0,
        saves: 0,
        comments: 0,
        follows: 0,
        promptTokens: 0,
        completionTokens: 0,
      },
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
  const selectedRow = rows.find((row) => row.udid === openLog) ?? null;

  const reload = useCallback(async () => {
    setMsg(null);
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
    let alive = true;
    let unlisten: (() => void) | undefined;
    void listenRiviuEvents((event) => {
      if (event.type !== "nurtureStatus") return;
      if (!alive) return;
      const st = event.status;
      setStatuses((prev) => {
        const next = prev.filter((x) => x.udid !== st.udid);
        next.push(st);
        return next;
      });
    })
      .then((fn) => {
        if (!alive) {
          fn();
          return;
        }
        unlisten = fn;
      })
      .catch(() => undefined);
    return () => {
      alive = false;
      unlisten?.();
    };
  }, []);

  const patch = <K extends keyof NurtureSettings>(key: K, value: NurtureSettings[K]) => {
    setSettings((prev) => (prev ? { ...prev, [key]: value } : prev));
  };

  type RateKey = "likeProb" | "saveProb" | "commentProb" | "followProb" | "frenzyProb";
  const patchRate = (key: RateKey, value: number) => {
    const bounded = Number.isFinite(value) ? Math.max(0, Math.min(100, Math.floor(value))) : 0;
    setSettings((prev) => (prev ? { ...prev, [key]: bounded } : prev));
  };

  const save = async (next?: NurtureSettings): Promise<boolean> => {
    const s = next ?? settings;
    if (!s) return false;
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
    if ((s.commentEnabled ?? true) && s.commentProb > 0 && !s.apiKey.trim()) {
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
    const runTargets = [...targets];
    setBusy(true);
    try {
      if (settings && !(await save({ ...settings, scheduleUdids: runTargets }))) return;
      await nurtureStart(runTargets);
      setStartedTargets((current) => [...new Set([...current, ...runTargets])]);
      await reload();
      if (pageSurface) setPageMode("monitor");
      else setTab("log");
    } catch (e) {
      setMsg(describeError(e));
    } finally {
      setBusy(false);
    }
  };

  const stop = async () => {
    setBusy(true);
    try {
      await nurtureStop(stopTargets);
      setStartedTargets([]);
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

  type PageMode = "setup" | "monitor";
  type SettingsTab = "behaviour" | "ai" | "comments" | "log";
  const visibleSettingsTabs: ReadonlyArray<readonly [SettingsTab, string]> = pageSurface
    ? [
        ["behaviour", "Hành vi"],
        ["ai", "AI"],
        ["comments", "Bình luận"],
      ]
    : [
        ["behaviour", "Hành vi"],
        ["ai", "AI"],
        ["comments", "Bình luận"],
        ["log", "Log"],
      ];

  const activatePageMode = (next: PageMode) => {
    setPageMode(next);
    document.getElementById(`nurture-page-tab-${next}`)?.focus();
  };
  const onPageTabKeyDown = (event: ReactKeyboardEvent<HTMLButtonElement>) => {
    let next: PageMode | null = null;
    if (event.key === "ArrowRight" || event.key === "End") next = "monitor";
    if (event.key === "ArrowLeft" || event.key === "Home") next = "setup";
    if (!next) return;
    event.preventDefault();
    activatePageMode(next);
  };
  const activateSettingsTab = (next: SettingsTab) => {
    setTab(next);
    document.getElementById(`nurture-settings-tab-${next}`)?.focus();
  };
  const onSettingsTabKeyDown = (
    event: ReactKeyboardEvent<HTMLButtonElement>,
    current: SettingsTab,
  ) => {
    const currentIndex = visibleSettingsTabs.findIndex(([key]) => key === current);
    let nextIndex: number | null = null;
    if (event.key === "Home") nextIndex = 0;
    if (event.key === "End") nextIndex = visibleSettingsTabs.length - 1;
    if (event.key === "ArrowRight") nextIndex = (currentIndex + 1) % visibleSettingsTabs.length;
    if (event.key === "ArrowLeft") {
      nextIndex = (currentIndex - 1 + visibleSettingsTabs.length) % visibleSettingsTabs.length;
    }
    if (nextIndex === null) return;
    event.preventDefault();
    activateSettingsTab(visibleSettingsTabs[nextIndex][0]);
  };

  const renderSettings = () => {
    if (!settings) return null;
    return (
      <>
        <div
          className="nurture-tabs"
          role="tablist"
          aria-label={pageSurface ? "Nhóm thiết lập Nuôi TikTok" : "Nuôi TikTok"}
        >
          {visibleSettingsTabs.map(([key, label]) => (
            <button
              key={key}
              id={`nurture-settings-tab-${key}`}
              type="button"
              role="tab"
              aria-selected={tab === key}
              aria-controls={`nurture-settings-panel-${key}`}
              tabIndex={tab === key ? 0 : -1}
              className={`nurture-tab${tab === key ? " is-on" : ""}`}
              onClick={() => setTab(key)}
              onKeyDown={(event) => onSettingsTabKeyDown(event, key)}
            >
              {label}
            </button>
          ))}
        </div>
        {visibleSettingsTabs
          .filter(([key]) => key !== "log")
          .map(([key]) => (
            <div
              key={key}
              id={`nurture-settings-panel-${key}`}
              role="tabpanel"
              aria-labelledby={`nurture-settings-tab-${key}`}
              hidden={tab !== key}
            >
              {tab === key && key === "ai" && (
                <NurtureAiTab
                  settings={settings}
                  patch={patch}
                  devices={devices}
                  targets={targets}
                  save={save}
                  onMessage={setMsg}
                />
              )}
              {tab === key && key === "behaviour" && (
                <NurtureBehaviourTab
                  settings={settings}
                  patch={patch}
                  patchRate={patchRate}
                  targets={targets}
                />
              )}
              {tab === key && key === "comments" && (
                <NurtureCommentsTab
                  live={anyRunning}
                  deviceLabel={(udid) => deviceLabel(devices, metas, udid)}
                />
              )}
            </div>
          ))}
      </>
    );
  };

  const actionControls = (
    <div className="nurture-float-actions">
      <button type="button" className="primary" disabled={busy || !targets.length} onClick={start}>
        Bắt đầu
      </button>
      <button type="button" className="danger" disabled={busy || !stopTargets.length} onClick={stop}>
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
  );

  return (
    <div
      className={pageSurface ? "nurture-workspace" : "nurture-float-layer"}
      role={pageSurface ? "region" : undefined}
      aria-label={pageSurface ? "Không gian Nuôi TikTok" : "Nuôi TikTok"}
    >
      <div
        className={pageSurface ? "nurture-workspace-inner" : "nurture-float"}
        style={pageSurface ? undefined : { transform: `translate(${pos.x}px, ${pos.y}px)` }}
      >
        {!pageSurface && (
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
        )}

        <div className={pageSurface ? "nurture-float-body nurture-workspace-body" : "nurture-float-body"}>
          {!settings ? (
            msg ? (
              <StatusNotice
                tone="error"
                action={(
                  <button type="button" onClick={() => void reload()}>
                    <IconRefresh size={15} /> Thử tải lại Nuôi TikTok
                  </button>
                )}
              >
                {msg}
              </StatusNotice>
            ) : (
              <LoadingState label="Đang tải Nuôi TikTok…" />
            )
          ) : (
            <>
              {!pageSurface && actionControls}

              {pageSurface && (
                <div className="nurture-page-tabs" role="tablist" aria-label="Chế độ Nuôi TikTok">
                  <button
                    id="nurture-page-tab-setup"
                    type="button"
                    role="tab"
                    aria-selected={pageMode === "setup"}
                    aria-controls="nurture-page-panel-setup"
                    tabIndex={pageMode === "setup" ? 0 : -1}
                    onClick={() => setPageMode("setup")}
                    onKeyDown={onPageTabKeyDown}
                  >
                    Thiết lập
                  </button>
                  <button
                    id="nurture-page-tab-monitor"
                    type="button"
                    role="tab"
                    aria-selected={pageMode === "monitor"}
                    aria-controls="nurture-page-panel-monitor"
                    tabIndex={pageMode === "monitor" ? 0 : -1}
                    onClick={() => setPageMode("monitor")}
                    onKeyDown={onPageTabKeyDown}
                  >
                    Theo dõi
                  </button>
                </div>
              )}

              {pageSurface && (
                <CommandBar
                  title={targets.length ? `${targets.length} máy trong phạm vi` : "Chưa có máy trong phạm vi"}
                  detail={pageMode === "monitor" ? "Theo dõi tiến độ hoặc dừng các máy trong phiên hiện tại." : anyRunning ? "Phiên đang chạy; có thể dừng hoặc lưu thay đổi." : "Kiểm tra thiết lập rồi bắt đầu phiên Nuôi TikTok."}
                  tone={targets.length ? "success" : "warning"}
                  actions={actionControls}
                />
              )}

              {pageSurface && (
                <div
                  id="nurture-page-panel-setup"
                  role="tabpanel"
                  aria-labelledby="nurture-page-tab-setup"
                  hidden={pageMode !== "setup"}
                >
                  {pageMode === "setup" && (
                    <div className="nurture-setup-grid">
                      <div className="nurture-setup-main">
                        {targetRef && profileConfig && (
                          <AutomationProfileControl
                            kind="nurture"
                            target={targetRef}
                            config={profileConfig}
                            defaultName="Hồ sơ Nuôi TikTok"
                          />
                        )}
                        {renderSettings()}
                      </div>
                      <SummaryRail
                        title="Kiểm tra trước khi chạy"
                        actions={(
                          <StatusChip tone={targets.length ? "success" : "warning"}>
                            {targets.length ? "Sẵn sàng" : "Thiếu phạm vi"}
                          </StatusChip>
                        )}
                      >
                        <dl className="nurture-review-list">
                          <div><dt>Thiết bị</dt><dd>{targets.length} máy</dd></div>
                          <div><dt>Khối lượng</dt><dd>{settings.numVideos * settings.numRounds} video/máy</dd></div>
                          <div><dt>Thích</dt><dd>{settings.likeEnabled === false ? "Tắt" : `${settings.likeProb}%`}</dd></div>
                          <div><dt>Lưu</dt><dd>{settings.saveEnabled === false ? "Tắt" : `${settings.saveProb}%`}</dd></div>
                          <div><dt>Bình luận</dt><dd>{settings.commentEnabled === false ? "Tắt" : `${settings.commentProb}%`}</dd></div>
                          <div><dt>Theo dõi</dt><dd>{settings.followEnabled === false ? "Tắt" : `${settings.followProb}%`}</dd></div>
                        </dl>
                      </SummaryRail>
                    </div>
                  )}
                </div>
              )}

              {!pageSurface && renderSettings()}

              <div
                id={pageSurface ? "nurture-page-panel-monitor" : "nurture-settings-panel-log"}
                className="nurture-live"
                role="tabpanel"
                aria-labelledby={
                  pageSurface ? "nurture-page-tab-monitor" : "nurture-settings-tab-log"
                }
                hidden={pageSurface ? pageMode !== "monitor" : tab !== "log"}
              >
                {((pageSurface && pageMode === "monitor") || (!pageSurface && tab === "log")) && (
                  <>
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
                      <span>Lưu / BL / Theo dõi</span>
                      <strong>
                        {totals.saves}/{totals.comments}/{totals.follows}
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
                  <div className="nurture-monitor-grid">
                    <div
                      className={`nurture-float-log${openLog ? " is-expanded" : ""}`}
                      aria-live="polite"
                      aria-label="Danh sách thiết bị Nuôi TikTok"
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
                          <strong>{deviceLabel(devices, metas, row.udid)}</strong>
                          <div className="grow" />
                          {/* The session numbers as labelled cells: the old
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
                              title="đã xác nhận / đã thử — video · tim · lưu · bình luận · theo dõi"
                            >
                              <b>{row.status.videosDone}</b>
                              <i>/{row.status.swipeAttempts}</i>
                              <em>v</em>
                              <b>{row.status.likes}</b>
                              <i>/{row.status.likeAttempts}</i>
                              <em>♥</em>
                              <b>{row.status.saves ?? 0}</b>
                              <i>/{row.status.saveAttempts ?? 0}</i>
                              <em>L</em>
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
                        {row.status && (
                          <p className="nurture-save-audit">
                            Lưu: {row.status.saves ?? 0} xác nhận · {row.status.saveAttempts ?? 0}{" "}
                            lần chạm · {row.status.saveNoops ?? 0} bỏ qua ·{" "}
                            {row.status.saveUncertain ?? 0} chưa chắc chắn
                          </p>
                        )}
                        {/* Outside the head `<button>` on purpose: a `progressbar` nested in a
                            button is neither, and the row head has to stay a plain control.
                            Gated on `row.status` so an idle-sweep row — which never ran a
                            session and has no target — does not draw a bar stuck at 0%. */}
                        {row.status && (
                          <NurtureDeviceProgress
                            status={row.status}
                            now={nowTick}
                            deviceName={deviceLabel(devices, metas, row.udid)}
                          />
                        )}
                        {!pageSurface && openLog === row.udid && (
                          <>
                            <details
                              className="nurture-technical-details"
                              aria-label="Chi tiết kỹ thuật thiết bị"
                            >
                              <summary>Chi tiết thiết bị</summary>
                              <code>{row.udid}</code>
                            </details>
                            <NurtureDeviceLog
                              udid={row.udid}
                              running={row.running}
                              presentStatus={statusVi}
                            />
                          </>
                        )}
                      </div>
                    ))}
                    </div>
                    {pageSurface && (
                      <aside className="nurture-device-detail" aria-label="Chi tiết thiết bị">
                        {selectedRow ? (
                          <>
                            <div className="nurture-device-detail-head">
                              <div>
                                <span>Thiết bị đang xem</span>
                                <strong>
                                  {deviceLabel(devices, metas, selectedRow.udid)}
                                </strong>
                              </div>
                              {selectedRow.status && <CleanupStatus status={selectedRow.status} />}
                            </div>
                            <p className="nurture-device-current">
                              {statusVi(selectedRow.message)}
                            </p>
                            <details
                              className="nurture-technical-details"
                              aria-label="Chi tiết kỹ thuật thiết bị"
                            >
                              <summary>Chi tiết thiết bị</summary>
                              <code>{selectedRow.udid}</code>
                            </details>
                            {selectedRow.status && (
                              <>
                                <NurtureDeviceProgress
                                  status={selectedRow.status}
                                  now={nowTick}
                                  deviceName={deviceLabel(devices, metas, selectedRow.udid)}
                                />
                                <dl className="nurture-device-counters">
                                  <div><dt>Video</dt><dd>{selectedRow.status.videosDone}/{selectedRow.status.swipeAttempts}</dd></div>
                                  <div><dt>Thích</dt><dd>{selectedRow.status.likes}/{selectedRow.status.likeAttempts}</dd></div>
                                  <div><dt>Lưu</dt><dd>{selectedRow.status.saves ?? 0}/{selectedRow.status.saveAttempts ?? 0}</dd></div>
                                  <div><dt>Bình luận</dt><dd>{selectedRow.status.comments}/{selectedRow.status.commentAttempts}</dd></div>
                                  <div><dt>Theo dõi</dt><dd>{selectedRow.status.follows}/{selectedRow.status.followAttempts}</dd></div>
                                </dl>
                              </>
                            )}
                            <NurtureDeviceLog
                              udid={selectedRow.udid}
                              running={selectedRow.running}
                              presentStatus={statusVi}
                            />
                          </>
                        ) : (
                          <div className="nurture-device-detail-empty">
                            Chọn một máy để xem nhật ký và chứng cứ.
                          </div>
                        )}
                      </aside>
                    )}
                  </div>
                    </>
                  ) : (
                    <p className="nurture-log-empty">Chưa có log nuôi TikTok.</p>
                  )}
                  </>
                )}
              </div>
              {msg && <p className="nurture-float-err">{msg}</p>}
            </>
          )}
        </div>
      </div>
    </div>
  );
}
