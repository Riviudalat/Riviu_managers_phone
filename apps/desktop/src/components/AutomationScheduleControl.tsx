import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { CalendarClock, Power, RefreshCw, Save } from "lucide-react";

import {
  automationScheduleCreate,
  automationScheduleList,
  automationScheduleUpdate,
} from "../api";
import { requestConfirm } from "../confirmStore";
import { describeError } from "../describeError";
import { scheduleDeviceLabel, scheduleErrorLabel, scheduleScope, type ScheduleScope } from "../scheduleScope";
import type {
  AutomationDefinition,
  AutomationSchedule,
  AutomationScheduleV1,
  DeviceInfo,
  DeviceMeta,
  JsonValue,
} from "../types";
import { NextRunsPreview, ScheduleScopeNotice } from "./ScheduleReview";
import { confirmScheduleApproval, needsGroups, useDeviceGroups, useScheduleTargets } from "../useScheduleScope";
import { EmptyState, LoadingState, StatusNotice } from "./States";

type IntervalScheduleV1 = {
  schemaVersion: 1;
  kind: "interval";
  everyMinutes: number;
};

const MIN_INTERVAL_MINUTES = 15;
const MAX_INTERVAL_MINUTES = 1440;

function intervalSchedule(everyMinutes: number): IntervalScheduleV1 {
  return { schemaVersion: 1, kind: "interval", everyMinutes };
}

function readIntervalMinutes(value: JsonValue): number | null {
  if (value === null || Array.isArray(value) || typeof value !== "object") return null;
  const schemaVersion = value.schemaVersion;
  const kind = value.kind;
  const everyMinutes = value.everyMinutes;
  return schemaVersion === 1 &&
    kind === "interval" &&
    typeof everyMinutes === "number" &&
    Number.isInteger(everyMinutes) &&
    everyMinutes >= MIN_INTERVAL_MINUTES &&
    everyMinutes <= MAX_INTERVAL_MINUTES
    ? everyMinutes
    : null;
}

function validInterval(minutes: number) {
  return (
    Number.isInteger(minutes) &&
    minutes >= MIN_INTERVAL_MINUTES &&
    minutes <= MAX_INTERVAL_MINUTES
  );
}

type Props = {
  profile: AutomationDefinition | null;
  /** The roster the approved device list is resolved against. */
  devices?: DeviceInfo[];
  metas?: Map<string, DeviceMeta>;
};

type ScheduleValues = {
  name: string;
  definitionRevision: number;
  enabled: boolean;
  schedule: AutomationScheduleV1;
  /** A newly approved device list; `null` keeps the stored approval. */
  approvedUdids: string[] | null;
};

/** Operator controls for recurring runs pinned to one immutable automation revision. */
export function AutomationScheduleControl({ profile, devices = [], metas }: Props) {
  const [schedules, setSchedules] = useState<AutomationSchedule[]>([]);
  const [loading, setLoading] = useState(false);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [newName, setNewName] = useState("");
  const [newMinutes, setNewMinutes] = useState(60);
  const loadGeneration = useRef(0);
  const profileId = profile?.id ?? null;
  const targetKeys = useMemo(
    () => [
      ...(profile ? [{ definitionId: profile.id, definitionRevision: profile.latestRevision }] : []),
      ...schedules,
    ],
    [profile, schedules],
  );
  const targets = useScheduleTargets(targetKeys);
  const groups = useDeviceGroups(needsGroups(targets.values()));
  const label = useCallback((udid: string) => scheduleDeviceLabel(udid, devices, metas), [devices, metas]);
  const scopeOf = (definitionId: string, revision: number, approved: string[] | null) =>
    scheduleScope(approved, targets.get(`${definitionId}:${revision}`), devices, groups);
  // What saving now would approve: the phones the profile's latest revision names.
  const newScope = profile ? scopeOf(profile.id, profile.latestRevision, null) : null;

  const load = useCallback(async () => {
    const generation = ++loadGeneration.current;
    if (!profileId) {
      setSchedules([]);
      setLoading(false);
      return;
    }
    setLoading(true);
    setError(null);
    try {
      const records = await automationScheduleList();
      if (loadGeneration.current !== generation) return;
      setSchedules(records.filter((record) => record.definitionId === profileId));
    } catch (cause) {
      if (loadGeneration.current !== generation) return;
      setError(describeError(cause));
    } finally {
      if (loadGeneration.current === generation) setLoading(false);
    }
  }, [profileId]);

  useEffect(() => {
    setNotice(null);
    setNewName("");
    setNewMinutes(60);
    void load();
  }, [load]);

  const create = useCallback(async (enabled: boolean) => {
    if (!profile) return;
    const name = newName.trim();
    if (!name) {
      setError("Tên lịch không được để trống.");
      return;
    }
    if (!validInterval(newMinutes)) {
      setError("Chu kỳ phải là số phút nguyên từ 15 đến 1.440 phút.");
      return;
    }
    if (!newScope || newScope.kind === "unknown") {
      setError("Chưa đọc được phạm vi máy của cấu hình. Tải lại rồi lưu lịch.");
      return;
    }
    if (newScope.kind === "missingGroup" || newScope.designated.length === 0) {
      setError("Phạm vi của cấu hình không có máy nào để duyệt.");
      return;
    }
    const approvedUdids = newScope.designated;
    setBusyId("new");
    setError(null);
    setNotice(null);
    try {
      const created = await automationScheduleCreate(
        name,
        profile.id,
        profile.latestRevision,
        enabled,
        intervalSchedule(newMinutes),
        approvedUdids,
      );
      setSchedules((current) => [...current, created]);
      setNewName("");
      setNewMinutes(60);
      setNotice(
        `${enabled ? "Đã lưu và bật lịch" : "Đã lưu lịch, đang tắt"} ở bản hồ sơ ${profile.latestRevision} · duyệt ${approvedUdids.length} máy.`,
      );
    } catch (cause) {
      setError(describeError(cause));
    } finally {
      setBusyId(null);
    }
  }, [newMinutes, newName, newScope, profile]);

  const update = useCallback(
    async (record: AutomationSchedule, values: ScheduleValues) => {
      setBusyId(record.id);
      setError(null);
      setNotice(null);
      try {
        const updated = await automationScheduleUpdate(
          record.id,
          record.revision,
          values.name,
          record.definitionId,
          values.definitionRevision,
          values.enabled,
          values.schedule,
          values.approvedUdids,
        );
        setSchedules((current) =>
          current.map((candidate) => (candidate.id === updated.id ? updated : candidate)),
        );
        return true;
      } catch (cause) {
        setError(describeError(cause));
        return false;
      } finally {
        setBusyId(null);
      }
    },
    [],
  );

  if (!profile) return null;

  return (
    <section className="automation-schedule-control" aria-label="Lịch tự động">
      <div className="automation-schedule-heading">
        <span className="automation-schedule-title">
          <CalendarClock size={16} aria-hidden />
          Lịch tự động
        </span>
        {!loading && (
          <button type="button" className="icon-btn" aria-label="Tải lại lịch" onClick={() => void load()}>
            <RefreshCw size={15} />
          </button>
        )}
      </div>

      {loading ? (
        <LoadingState label="Đang tải lịch…" />
      ) : error && schedules.length === 0 ? (
        <StatusNotice
          tone="error"
          action={
            <button type="button" onClick={() => void load()}>
              <RefreshCw size={15} /> Thử tải lại lịch
            </button>
          }
        >
          {error}
        </StatusNotice>
      ) : (
        <>
          {schedules.length === 0 ? (
            <EmptyState title="Chưa có lịch cho hồ sơ này" hint="Đặt tên và chu kỳ bên dưới, rồi bấm Lưu hoặc Lưu & bật." compact />
          ) : (
            <div className="automation-schedule-list">
              {schedules.map((record) => (
                <ScheduleRow
                  key={`${record.id}:${record.revision}`}
                  record={record}
                  currentProfileRevision={profile.latestRevision}
                  busy={busyId === record.id}
                  scope={scopeOf(record.definitionId, record.definitionRevision, record.approvedUdids)}
                  label={label}
                  onUpdate={update}
                />
              ))}
            </div>
          )}

          <div className="automation-schedule-create" role="group" aria-label="Tạo lịch mới">
            <label>
              <span>Tên lịch</span>
              <input
                aria-label="Tên lịch mới"
                value={newName}
                disabled={busyId !== null}
                placeholder="Ví dụ: Ca buổi sáng"
                onChange={(event) => setNewName(event.currentTarget.value)}
              />
            </label>
            <label>
              <span>Chu kỳ (phút)</span>
              <input
                aria-label="Chu kỳ lịch mới (phút)"
                type="number"
                min={MIN_INTERVAL_MINUTES}
                max={MAX_INTERVAL_MINUTES}
                step={1}
                value={newMinutes}
                disabled={busyId !== null}
                onChange={(event) => setNewMinutes(Number(event.currentTarget.value))}
              />
            </label>
            <div className="schedule-save-actions">
              <button type="button" aria-label="Lưu lịch, chưa bật" disabled={busyId !== null} onClick={() => void create(false)}>
                <Save size={15} /> Lưu
              </button>
              <button type="button" className="primary" aria-label="Lưu & bật lịch" disabled={busyId !== null} onClick={() => void create(true)}>
                <Power size={15} /> Lưu & bật
              </button>
            </div>
          </div>
          {newScope && newScope.kind !== "unknown" && newScope.kind !== "missingGroup" && (
            <p className="schedule-scope-line">
              Lưu sẽ duyệt {newScope.designated.length} máy: {newScope.designated.map(label).join(", ") || "không có máy nào"}.
              Nhóm đổi sau đó sẽ dừng lịch tới khi duyệt lại.
            </p>
          )}
          {newScope?.kind === "missingGroup" && (
            <StatusNotice tone="error">Nhóm máy của cấu hình không còn; chọn phạm vi khác rồi lưu bản hồ sơ mới.</StatusNotice>
          )}
          <NextRunsPreview nextDueAt={null} everyMinutes={newMinutes} enabled={false} />

          {error && (
            <StatusNotice
              tone="error"
              action={
                <button type="button" onClick={() => void load()}>
                  <RefreshCw size={15} /> Tải lại sau xung đột
                </button>
              }
            >
              {error}
            </StatusNotice>
          )}
          {notice && <StatusNotice tone="success">{notice}</StatusNotice>}
        </>
      )}
    </section>
  );
}

function ScheduleRow({
  record,
  currentProfileRevision,
  busy,
  scope,
  label,
  onUpdate,
}: {
  record: AutomationSchedule;
  currentProfileRevision: number;
  busy: boolean;
  scope: ScheduleScope;
  label: (udid: string) => string;
  onUpdate: (record: AutomationSchedule, values: ScheduleValues) => Promise<boolean>;
}) {
  const parsedMinutes = readIntervalMinutes(record.schedule);
  const [name, setName] = useState(record.name);
  const [minutes, setMinutes] = useState(parsedMinutes ?? 60);
  const canRetarget = record.definitionRevision !== currentProfileRevision;
  // Switching on needs a scope that matches what was approved; switching off never waits.
  const enableBlocked = !record.enabled && scope.kind !== "approved";
  const lastError = scheduleErrorLabel(record.lastErrorCode);

  const save = async () => {
    const trimmedName = name.trim();
    if (!trimmedName || !validInterval(minutes)) return;
    await onUpdate(record, {
      name: trimmedName,
      definitionRevision: record.definitionRevision,
      enabled: record.enabled,
      schedule: intervalSchedule(minutes),
      approvedUdids: null,
    });
  };

  const retarget = async () => {
    const confirmed = await requestConfirm({
      title: `Chuyển lịch sang bản hồ sơ ${currentProfileRevision}?`,
      message: `${record.name} đang ghim bản ${record.definitionRevision}. Lần chạy kế tiếp sẽ dùng chính xác bản ${currentProfileRevision}.`,
      confirmLabel: `Áp dụng bản ${currentProfileRevision}`,
      cancelLabel: "Giữ bản đang ghim",
    });
    if (!confirmed) return;
    await onUpdate(record, {
      name: record.name,
      definitionRevision: currentProfileRevision,
      enabled: record.enabled,
      schedule: intervalSchedule(parsedMinutes!),
      approvedUdids: null,
    });
  };

  const approve = async (udids: string[]) => {
    if (parsedMinutes === null || !(await confirmScheduleApproval(record.name, udids, label))) return;
    await onUpdate(record, {
      name: record.name,
      definitionRevision: record.definitionRevision,
      enabled: record.enabled,
      schedule: intervalSchedule(parsedMinutes),
      approvedUdids: udids,
    });
  };

  return (
    <article className="automation-schedule-row" role="group" aria-label={`Lịch ${record.name}`}>
      <div className="automation-schedule-row-main">
        <label>
          <span>Tên</span>
          <input
            aria-label={`Tên lịch ${record.name}`}
            value={name}
            disabled={busy || parsedMinutes === null}
            onChange={(event) => setName(event.currentTarget.value)}
          />
        </label>
        <label>
          <span>Chu kỳ (phút)</span>
          <input
            aria-label={`Chu kỳ ${record.name} (phút)`}
            type="number"
            min={MIN_INTERVAL_MINUTES}
            max={MAX_INTERVAL_MINUTES}
            step={1}
            value={minutes}
            disabled={busy || parsedMinutes === null}
            onChange={(event) => setMinutes(Number(event.currentTarget.value))}
          />
        </label>
        <span className={`automation-schedule-state ${record.enabled ? "enabled" : "disabled"}`}>
          {record.enabled ? "Đang bật" : "Đang tắt"}
        </span>
        <button
          type="button"
          className="icon-btn"
          aria-label={`Lưu lịch ${record.name}`}
          title="Lưu thay đổi"
          disabled={busy || parsedMinutes === null || !name.trim() || !validInterval(minutes)}
          onClick={() => void save()}
        >
          <Save size={15} />
        </button>
        <button
          type="button"
          className="icon-btn"
          aria-label={`${record.enabled ? "Tắt" : "Bật"} lịch ${record.name}`}
          title={record.enabled ? "Tắt lịch" : enableBlocked ? "Duyệt danh sách máy trước khi bật" : "Bật lịch"}
          disabled={busy || parsedMinutes === null || enableBlocked}
          onClick={() =>
            void onUpdate(record, {
              name: record.name,
              definitionRevision: record.definitionRevision,
              enabled: !record.enabled,
              schedule: intervalSchedule(parsedMinutes!),
              approvedUdids: null,
            })
          }
        >
          <Power size={15} />
        </button>
      </div>

      {parsedMinutes === null && (
        <StatusNotice tone="warning">Định dạng lịch này chưa được hỗ trợ.</StatusNotice>
      )}
      <ScheduleScopeNotice
        scope={scope}
        approved={record.approvedUdids}
        label={label}
        busy={busy || parsedMinutes === null}
        onApprove={(udids) => void approve(udids)}
      />
      {lastError && <p className="hint">Lần gần nhất: {lastError}</p>}
      {parsedMinutes !== null && (
        <NextRunsPreview nextDueAt={record.nextDueAt} everyMinutes={parsedMinutes} enabled={record.enabled} compact />
      )}
      <div className="automation-schedule-row-meta">
        <span>Ghim bản hồ sơ {record.definitionRevision}</span>
        {canRetarget && (
          <button
            type="button"
            disabled={busy || parsedMinutes === null}
            onClick={() => void retarget()}
          >
            Áp dụng bản hồ sơ {currentProfileRevision}
          </button>
        )}
        <details role="group" aria-label={`Chi tiết kỹ thuật lịch ${record.name}`}>
          <summary>Chi tiết</summary>
          <code>{record.id}</code>
          <span>Revision lịch {record.revision}</span>
        </details>
      </div>
    </article>
  );
}
