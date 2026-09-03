import { useCallback, useEffect, useRef, useState } from "react";
import { CalendarClock, Plus, Power, RefreshCw, Save } from "lucide-react";

import {
  automationScheduleCreate,
  automationScheduleList,
  automationScheduleUpdate,
} from "../api";
import { requestConfirm } from "../confirmStore";
import { describeError } from "../describeError";
import type {
  AutomationDefinition,
  AutomationSchedule,
  AutomationScheduleV1,
  JsonValue,
} from "../types";
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
};

/** Operator controls for recurring runs pinned to one immutable automation revision. */
export function AutomationScheduleControl({ profile }: Props) {
  const [schedules, setSchedules] = useState<AutomationSchedule[]>([]);
  const [loading, setLoading] = useState(false);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [newName, setNewName] = useState("");
  const [newMinutes, setNewMinutes] = useState(60);
  const loadGeneration = useRef(0);
  const profileId = profile?.id ?? null;

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

  const create = useCallback(async () => {
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
    setBusyId("new");
    setError(null);
    setNotice(null);
    try {
      const created = await automationScheduleCreate(
        name,
        profile.id,
        profile.latestRevision,
        true,
        intervalSchedule(newMinutes),
      );
      setSchedules((current) => [...current, created]);
      setNewName("");
      setNewMinutes(60);
      setNotice(`Đã tạo lịch ở bản hồ sơ ${profile.latestRevision}.`);
    } catch (cause) {
      setError(describeError(cause));
    } finally {
      setBusyId(null);
    }
  }, [newMinutes, newName, profile]);

  const update = useCallback(
    async (
      record: AutomationSchedule,
      values: {
        name: string;
        definitionRevision: number;
        enabled: boolean;
        schedule: AutomationScheduleV1;
      },
    ) => {
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
            <EmptyState title="Chưa có lịch cho hồ sơ này" compact />
          ) : (
            <div className="automation-schedule-list">
              {schedules.map((record) => (
                <ScheduleRow
                  key={`${record.id}:${record.revision}`}
                  record={record}
                  currentProfileRevision={profile.latestRevision}
                  busy={busyId === record.id}
                  onUpdate={update}
                />
              ))}
            </div>
          )}

          <div className="automation-schedule-create" aria-label="Tạo lịch mới">
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
            <button type="button" disabled={busyId !== null} onClick={() => void create()}>
              <Plus size={15} /> Tạo lịch
            </button>
          </div>

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
  onUpdate,
}: {
  record: AutomationSchedule;
  currentProfileRevision: number;
  busy: boolean;
  onUpdate: (
    record: AutomationSchedule,
    values: {
      name: string;
      definitionRevision: number;
      enabled: boolean;
      schedule: AutomationScheduleV1;
    },
  ) => Promise<boolean>;
}) {
  const parsedMinutes = readIntervalMinutes(record.schedule);
  const [name, setName] = useState(record.name);
  const [minutes, setMinutes] = useState(parsedMinutes ?? 60);
  const canRetarget = record.definitionRevision !== currentProfileRevision;

  const save = async () => {
    const trimmedName = name.trim();
    if (!trimmedName || !validInterval(minutes)) return;
    await onUpdate(record, {
      name: trimmedName,
      definitionRevision: record.definitionRevision,
      enabled: record.enabled,
      schedule: intervalSchedule(minutes),
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
          title={record.enabled ? "Tắt lịch" : "Bật lịch"}
          disabled={busy || parsedMinutes === null}
          onClick={() =>
            void onUpdate(record, {
              name: record.name,
              definitionRevision: record.definitionRevision,
              enabled: !record.enabled,
              schedule: intervalSchedule(parsedMinutes!),
            })
          }
        >
          <Power size={15} />
        </button>
      </div>

      {parsedMinutes === null && (
        <StatusNotice tone="warning">Định dạng lịch này chưa được hỗ trợ.</StatusNotice>
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
