import { useEffect, useRef, useState } from "react";
import { RefreshCw } from "lucide-react";

import {
  deviceBaselineApply,
  deviceBaselineGetConfig,
  deviceBaselineRead,
  deviceBaselineSaveConfig,
  listGroups,
} from "../api";
import { resolveAutomationTarget } from "../automationTargets";
import { requestConfirm } from "../confirmStore";
import { describeError } from "../describeError";
import { LoadingState, StatusNotice } from "../components/States";
import { TargetSelector } from "../components/TargetSelector";
import { FormSection, StatusChip, type StatusTone } from "../components/WorkspacePrimitives";
import type {
  BaselineOutcome,
  BaselineSetting,
  BaselineStatus,
  DeviceBaselineConfig,
  DeviceBaselineReading,
  DeviceBaselineResult,
  DeviceGroup,
  DeviceInfo,
  TargetRef,
} from "../types";

/** The closed catalogue, in display order, with the exact keys each one writes. */
const BASELINE_SETTINGS: { id: BaselineSetting; label: string; hint: string }[] = [
  {
    id: "lockScreenDisabled",
    label: "Tắt khóa màn hình",
    hint: "locksettings set-disabled true, rồi wm dismiss-keyguard. Máy có mã PIN/mật khẩu/hình vẽ chỉ được báo “Cần làm tay”, không thử mở.",
  },
  {
    id: "autoRotateOff",
    label: "Tắt tự xoay màn hình",
    hint: "accelerometer_rotation = 0 và user_rotation = 0 (dọc).",
  },
  {
    id: "stayAwakeWhileCharging",
    label: "Luôn sáng khi đang sạc",
    hint: "stay_on_while_plugged_in = 7 (sạc AC, USB, không dây). Màn hình không tắt nên không tự khóa lại.",
  },
  {
    id: "screenOffTimeoutMax",
    label: "Tắt màn hình sau 30 phút",
    hint: "screen_off_timeout = 1800000, mức dài nhất Cài đặt của máy cho chọn.",
  },
  {
    id: "animationsOff",
    label: "Tắt hiệu ứng chuyển động",
    hint: "window/transition/animator scale = 0. Đổi nhịp màn hình; thử trên một máy trước khi áp cho cả đội.",
  },
];

const STATUS: Record<BaselineStatus, { label: string; tone: StatusTone }> = {
  ok: { label: "Đúng chuẩn", tone: "success" },
  drift: { label: "Chưa đúng", tone: "warning" },
  unknown: { label: "Chưa rõ", tone: "neutral" },
  needsManual: { label: "Cần làm tay", tone: "error" },
};

const OUTCOME: Record<BaselineOutcome, { label: string; tone: StatusTone }> = {
  applied: { label: "Đã áp dụng", tone: "success" },
  alreadyOk: { label: "Đã đúng chuẩn", tone: "success" },
  needsManual: { label: "Cần làm tay", tone: "warning" },
  refusedBusy: { label: "Máy đang bận", tone: "warning" },
  failed: { label: "Lỗi", tone: "error" },
  unsupported: { label: "Chưa hỗ trợ", tone: "neutral" },
};

/** Phones run a few at a time: each apply is several adb round trips on its own lease. */
const APPLY_CONCURRENCY = 4;

type ReadState = { state: "loading" } | { state: "done"; reading: DeviceBaselineReading } | { state: "error"; message: string };

function sameConfig(left: DeviceBaselineConfig, right: DeviceBaselineConfig) {
  return left.autoApplyOnConnect === right.autoApplyOnConnect
    && [...left.settings].sort().join(",") === [...right.settings].sort().join(",");
}

async function eachBounded<T>(items: T[], limit: number, run: (item: T) => Promise<void>) {
  let next = 0;
  const worker = async () => {
    while (next < items.length) {
      const item = items[next++];
      await run(item);
    }
  };
  await Promise.all(Array.from({ length: Math.min(limit, items.length) }, worker));
}

/** "Cài đặt máy": the fleet baseline of Android system settings, checked and applied by scope. */
export function DeviceSettingsPage({ devices }: { devices: DeviceInfo[] }) {
  const [saved, setSaved] = useState<DeviceBaselineConfig | null>(null);
  const [draft, setDraft] = useState<DeviceBaselineConfig | null>(null);
  const [configError, setConfigError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [groups, setGroups] = useState<DeviceGroup[]>([]);
  const [targetRef, setTargetRef] = useState<TargetRef>({ type: "all" });
  const [readings, setReadings] = useState<Map<string, ReadState>>(new Map());
  const [results, setResults] = useState<Map<string, DeviceBaselineResult>>(new Map());
  const [running, setRunning] = useState<"read" | "apply" | null>(null);
  const configTicket = useRef(0);
  const runTicket = useRef(0);

  const loadConfig = async () => {
    const ticket = ++configTicket.current;
    try {
      const config = await deviceBaselineGetConfig();
      if (ticket !== configTicket.current) return;
      setConfigError(null);
      setSaved(config);
      setDraft(config);
    } catch (error) {
      if (ticket === configTicket.current) setConfigError(describeError(error));
    }
  };

  useEffect(() => {
    void loadConfig();
    void listGroups().then(setGroups).catch(() => setGroups([]));
    return () => {
      configTicket.current += 1;
      runTicket.current += 1;
    };
  }, []);

  const targets = resolveAutomationTarget(targetRef, devices, groups);
  const targetDevices = devices.filter((device) => targets.includes(device.udid));
  const android = targetDevices.filter((device) => device.platform === "android");
  const dirty = Boolean(saved && draft && !sameConfig(saved, draft));
  const plan = saved?.settings ?? [];

  const save = async () => {
    if (!draft) return;
    setSaving(true);
    setConfigError(null);
    try {
      const stored = await deviceBaselineSaveConfig(draft);
      setSaved(stored);
      setDraft(stored);
    } catch (error) {
      setConfigError(describeError(error));
    } finally {
      setSaving(false);
    }
  };

  const check = async () => {
    const ticket = ++runTicket.current;
    setRunning("read");
    setReadings(new Map(android.map((device) => [device.udid, { state: "loading" } as ReadState])));
    await eachBounded(android, APPLY_CONCURRENCY, async (device) => {
      let next: ReadState;
      try {
        next = { state: "done", reading: await deviceBaselineRead(device.udid) };
      } catch (error) {
        next = { state: "error", message: describeError(error) };
      }
      if (ticket === runTicket.current) setReadings((current) => new Map(current).set(device.udid, next));
    });
    if (ticket === runTicket.current) setRunning(null);
  };

  const apply = async () => {
    if (plan.length === 0 || android.length === 0) return;
    const names = plan.map((id) => BASELINE_SETTINGS.find((item) => item.id === id)?.label ?? id).join(", ");
    const confirmed = await requestConfirm({
      title: `Áp dụng chuẩn cho ${android.length} máy Android?`,
      message: `${names}. Máy đang chạy việc khác sẽ được bỏ qua (không chiếm quyền); máy có mã khóa chỉ được báo cần làm tay.`,
      confirmLabel: "Áp dụng",
    });
    if (!confirmed) return;
    const ticket = ++runTicket.current;
    setRunning("apply");
    setResults(new Map());
    await eachBounded(android, APPLY_CONCURRENCY, async (device) => {
      let result: DeviceBaselineResult;
      try {
        result = await deviceBaselineApply(device.udid, plan);
      } catch (error) {
        result = { udid: device.udid, outcome: "failed", items: [], detail: describeError(error) };
      }
      if (ticket === runTicket.current) setResults((current) => new Map(current).set(device.udid, result));
    });
    if (ticket === runTicket.current) setRunning(null);
  };

  const toggleSetting = (id: BaselineSetting, on: boolean) => {
    if (!draft) return;
    const settings = on ? [...draft.settings, id] : draft.settings.filter((item) => item !== id);
    setDraft({ ...draft, settings });
  };

  return (
    <div className="admin-workspace device-settings-workspace">
      <main className="admin-main">
        <div className="admin-toolbar">
          <div className="admin-toolbar-copy">
            <strong>Chuẩn cài đặt máy</strong>
            <span>Đọc, áp dụng và kiểm lại cài đặt hệ thống Android theo phạm vi đã chọn</span>
          </div>
          <div className="admin-toolbar-actions">
            <button type="button" className="ghost" onClick={() => void check()} disabled={running !== null || android.length === 0}>
              <RefreshCw size={15} aria-hidden="true" />
              {running === "read" ? "Đang kiểm tra…" : "Kiểm tra"}
            </button>
            <button type="button" className="primary" onClick={() => void apply()}
              disabled={running !== null || android.length === 0 || plan.length === 0 || dirty}
              title={dirty ? "Lưu chuẩn trước khi áp dụng" : undefined}>
              {running === "apply" ? "Đang áp dụng…" : `Áp dụng cho ${android.length} máy`}
            </button>
          </div>
        </div>

        <FormSection
          title="Chuẩn cài đặt"
          description="Chỉ cài đặt hệ thống an toàn; không đụng tài khoản, mạng, quyền gỡ lỗi USB hay dữ liệu ứng dụng."
          actions={draft && (
            <>
              <span className="settings-save-state" data-dirty={dirty}>{dirty ? "Chưa lưu" : "Đã lưu"}</span>
              <button type="button" className="ghost" onClick={() => saved && setDraft(saved)} disabled={!dirty || saving}>Bỏ thay đổi</button>
              <button type="button" className="primary" onClick={() => void save()} disabled={!dirty || saving}>
                {saving ? "Đang lưu…" : "Lưu chuẩn"}
              </button>
            </>
          )}
        >
          {!draft && !configError && <LoadingState label="Đang tải chuẩn cài đặt…" />}
          {configError && (
            <StatusNotice tone="error" action={<button type="button" className="ghost" onClick={() => void loadConfig()}>Thử lại</button>}>
              Không tải hoặc lưu được chuẩn cài đặt: {configError}
            </StatusNotice>
          )}
          {draft && (
            <div className="device-settings-catalogue">
              {BASELINE_SETTINGS.map((item) => (
                <label key={item.id} className="agent-toggle device-settings-item">
                  <input type="checkbox" checked={draft.settings.includes(item.id)}
                    onChange={(event) => toggleSetting(item.id, event.target.checked)} />
                  <span>
                    <strong>{item.label}</strong>
                    <small className="hint">{item.hint}</small>
                  </span>
                </label>
              ))}
              <label className="agent-toggle device-settings-item">
                <input type="checkbox" checked={draft.autoApplyOnConnect}
                  onChange={(event) => setDraft({ ...draft, autoApplyOnConnect: event.target.checked })} />
                <span>
                  <strong>Tự áp dụng khi máy kết nối</strong>
                  <small className="hint">Mỗi lần máy Android cắm lại, Riviu áp chuẩn đã lưu trước khi nhận việc; kết quả ghi vào nhật ký thao tác.</small>
                </span>
              </label>
            </div>
          )}
        </FormSection>

        <FormSection title="Phạm vi thiết bị" description="Một máy, một nhóm hoặc toàn bộ. iPhone hiển thị “Chưa hỗ trợ” và không bị thay đổi.">
          <TargetSelector devices={devices} groups={groups} selected={[]} onChange={() => undefined}
            targetRef={targetRef} onTargetRefChange={setTargetRef} label="Phạm vi thiết bị" />
        </FormSection>

        <FormSection title="Kết quả theo máy" description="Trạng thái đọc từ máy; “Chưa rõ” nghĩa là máy không trả lời được, không phải đã đúng.">
          {targetDevices.length === 0 ? (
            <p className="hint">Chưa có máy trong phạm vi đã chọn.</p>
          ) : (
            <table className="device-settings-table">
              <thead>
                <tr>
                  <th scope="col">Máy</th>
                  {BASELINE_SETTINGS.map((item) => <th key={item.id} scope="col">{item.label}</th>)}
                  <th scope="col">Lần áp dụng</th>
                </tr>
              </thead>
              <tbody>
                {targetDevices.map((device) => (
                  <DeviceRow key={device.udid} device={device} read={readings.get(device.udid)} result={results.get(device.udid)} />
                ))}
              </tbody>
            </table>
          )}
        </FormSection>
      </main>
    </div>
  );
}

function DeviceRow({ device, read, result }: { device: DeviceInfo; read?: ReadState; result?: DeviceBaselineResult }) {
  if (device.platform !== "android") {
    return (
      <tr data-udid={device.udid}>
        <th scope="row">{device.name}</th>
        <td colSpan={BASELINE_SETTINGS.length + 1}><StatusChip tone="neutral">Chưa hỗ trợ</StatusChip> iOS chưa có Cài đặt máy</td>
      </tr>
    );
  }
  const readError = read?.state === "error" ? read.message : read?.state === "done" ? read.reading.error : undefined;
  return (
    <tr data-udid={device.udid}>
      <th scope="row">{device.name}</th>
      {BASELINE_SETTINGS.map((item) => {
        if (!read) return <td key={item.id}><span className="hint">Chưa kiểm tra</span></td>;
        if (read.state === "loading") return <td key={item.id}><span className="hint">Đang đọc…</span></td>;
        const reading = read.state === "done" ? read.reading.settings.find((entry) => entry.setting === item.id) : undefined;
        // A failed read is "Chưa rõ" for every setting -- never a guess that it is fine.
        const status = STATUS[reading?.status ?? "unknown"];
        return (
          <td key={item.id} title={[reading?.observed, reading?.detail, readError].filter(Boolean).join(" · ") || undefined}>
            <StatusChip tone={status.tone}>{status.label}</StatusChip>
          </td>
        );
      })}
      <td>
        {result ? (
          <span title={[result.detail, ...result.items.filter((item) => item.detail).map((item) => item.detail)].filter(Boolean).join(" · ") || undefined}>
            <StatusChip tone={OUTCOME[result.outcome].tone}>{OUTCOME[result.outcome].label}</StatusChip>
            {result.detail && <small className="hint"> {result.detail}</small>}
            {result.items.filter((item) => item.outcome === "needsManual" || item.outcome === "failed").map((item) => (
              <small key={item.setting} className="hint"> {item.detail ?? OUTCOME[item.outcome].label}</small>
            ))}
          </span>
        ) : readError ? <small className="hint">{readError}</small> : <span className="hint">—</span>}
      </td>
    </tr>
  );
}
