import { useCallback, useEffect, useState } from "react";
import {
  Plus,
  Play,
  Save,
  CalendarClock,
  RefreshCw,
  Trash2,
} from "lucide-react";
import {
  appWorkflowList,
  appWorkflowRun,
  type AppWorkflowSummary,
} from "../appWorkflow";
import {
  operatorList,
  operatorSave,
  operatorArchive,
  type OperatorRecord,
} from "../operatorRecords";
import type { DeviceInfo } from "../types";
import { describeError } from "../describeError";
import { requestConfirm } from "../confirmStore";
import { invoke } from "@tauri-apps/api/core";

export function SavedTasksPage({ devices }: { devices: DeviceInfo[] }) {
  const [rows, setRows] = useState<OperatorRecord[]>([]),
    [apps, setApps] = useState<AppWorkflowSummary[]>([]),
    [name, setName] = useState(""),
    [appId, setAppId] = useState(""),
    [udids, setUdids] = useState<string[]>([]),
    [editing, setEditing] = useState<OperatorRecord | null>(null),
    [form, setForm] = useState(false),
    [busy, setBusy] = useState(false),
    [message, setMessage] = useState(""),
    [error, setError] = useState<string | null>(null);
  const load = useCallback(async () => {
    try {
      const [tasks, applications] = await Promise.all([
        operatorList("savedTask"),
        appWorkflowList(),
      ]);
      setRows(tasks);
      setApps(applications);
    } catch (cause) {
      setError(describeError(cause));
    }
  }, []);
  useEffect(() => {
    void load();
  }, [load]);
  const run = async (record: OperatorRecord) => {
    if (
      !(await requestConfirm({
        title: "Chạy tác vụ đã lưu?",
        message: record.name,
        confirmLabel: "Chạy",
      }))
    )
      return;
    setBusy(true);
    try {
      const target = record.data.target as {
        type: "explicit";
        udids: string[];
      };
      const result = await appWorkflowRun(
        String(record.data.appId),
        Number(record.data.appRevision),
        target,
      );
      setMessage(`Đã tạo lượt chạy ${result.run.id}`);
      setError(null);
    } catch (cause) {
      setError(describeError(cause));
    } finally {
      setBusy(false);
    }
  };
  return (
    <section className="operator-records">
      <header className="operator-toolbar">
        <strong>{rows.length} tác vụ đã lưu</strong>
        <div className="grow" />
        <button type="button" onClick={() => void load()}>
          <RefreshCw size={16} />
        </button>
        <button
          type="button"
          className="primary"
          onClick={() => {
            setEditing(null);
            setName("");
            setAppId("");
            setUdids([]);
            setForm(true);
          }}
        >
          <Plus size={16} />
          Tác vụ mới
        </button>
      </header>
      {error && <p role="alert">{error}</p>}
      {message && <p role="status">{message}</p>}
      <table>
        <thead>
          <tr>
            <th>Tên tác vụ</th>
            <th>Ứng dụng</th>
            <th>Bản ghim</th>
            <th>Thao tác</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((record) => (
            <tr key={record.id}>
              <td>{record.name}</td>
              <td>
                {apps.find((a) => a.id === record.data.appId)?.name ??
                  String(record.data.appId)}
              </td>
              <td>{String(record.data.appRevision)}</td>
              <td>
                <button
                  type="button"
                  disabled={busy}
                  onClick={() => void run(record)}
                >
                  <Play size={15} />
                  Chạy
                </button>
                <button
                  type="button"
                  onClick={() => {
                    setEditing(record);
                    setName(record.name);
                    setAppId(String(record.data.appId));
                    const target = record.data.target as { udids?: string[] };
                    setUdids(target.udids ?? []);
                    setForm(true);
                  }}
                >
                  Chỉnh sửa
                </button>
                <button
                  type="button"
                  title="Lập lịch mỗi giờ"
                  disabled={busy}
                  onClick={async () => {
                    if (
                      !(await requestConfirm({
                        title: "Tạo lịch chạy mỗi giờ?",
                        message: `${record.name} · dùng phiên bản và máy đã lưu.`,
                        confirmLabel: "Tạo lịch",
                      }))
                    )
                      return;
                    setBusy(true);
                    try {
                      await invoke("app_workflow_schedule", {
                        id: record.data.appId,
                        revision: record.data.appRevision,
                        target: record.data.target,
                        name: record.name,
                        everyMinutes: 60,
                      });
                      setMessage(
                        "Đã tạo lịch 60 phút. Mở Lịch chạy để quản lý.",
                      );
                    } catch (cause) {
                      setError(describeError(cause));
                    } finally {
                      setBusy(false);
                    }
                  }}
                >
                  <CalendarClock size={15} />
                </button>
                <button
                  type="button"
                  title="Lưu trữ tác vụ"
                  onClick={async () => {
                    if (
                      await requestConfirm({
                        title: "Lưu trữ tác vụ?",
                        message: record.name,
                        confirmLabel: "Lưu trữ",
                      })
                    ) {
                      try {
                        await operatorArchive(record);
                        await load();
                      } catch (cause) {
                        setError(describeError(cause));
                      }
                    }
                  }}
                >
                  <Trash2 size={15} />
                </button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      {!rows.length && (
        <p className="operator-empty">
          Lưu tác vụ để giữ ứng dụng, phiên bản và phạm vi thiết bị cho lần chạy
          sau.
        </p>
      )}
      {form && (
        <aside className="operator-record-editor">
          <h3>{editing ? "Chỉnh tác vụ" : "Tác vụ mới"}</h3>
          <label>
            Tên
            <input value={name} onChange={(e) => setName(e.target.value)} />
          </label>
          <label>
            Ứng dụng
            <select value={appId} onChange={(e) => setAppId(e.target.value)}>
              <option value="">Chọn ứng dụng…</option>
              {apps.map((app) => (
                <option key={app.id} value={app.id}>
                  {app.name} · bản {app.latestRevision}
                </option>
              ))}
            </select>
          </label>
          <fieldset>
            <legend>Máy thực hiện</legend>
            {devices.map((device) => (
              <label key={device.udid}>
                <input
                  type="checkbox"
                  checked={udids.includes(device.udid)}
                  onChange={(e) =>
                    setUdids((current) =>
                      e.target.checked
                        ? [...current, device.udid]
                        : current.filter((id) => id !== device.udid),
                    )
                  }
                />
                {device.name}
              </label>
            ))}
          </fieldset>
          <button
            type="button"
            disabled={!name.trim() || !appId || !udids.length || busy}
            className="primary"
            onClick={async () => {
              const app = apps.find((a) => a.id === appId);
              if (!app) return;
              setBusy(true);
              try {
                await operatorSave({
                  id: editing?.id ?? crypto.randomUUID(),
                  kind: "savedTask",
                  name,
                  expectedRevision: editing?.revision ?? null,
                  data: {
                    appId,
                    appRevision: app.latestRevision,
                    target: { type: "explicit", udids },
                    inputs: {},
                  },
                });
                setForm(false);
                await load();
              } catch (cause) {
                setError(describeError(cause));
              } finally {
                setBusy(false);
              }
            }}
          >
            <Save size={15} />
            Lưu tác vụ
          </button>
          <button type="button" onClick={() => setForm(false)}>
            Đóng
          </button>
        </aside>
      )}
    </section>
  );
}
