import { useCallback, useEffect, useState } from "react";
import { Download, Plus, RefreshCw, Save, Trash2, X } from "lucide-react";
import {
  operatorArchive,
  operatorList,
  operatorSave,
  operatorNetworkProbe,
  operatorNetworkApply,
  operatorImport,
  type OperatorRecord,
  type OperatorRecordKind,
} from "../operatorRecords";
import { describeError } from "../describeError";
import { requestConfirm } from "../confirmStore";
import type { DeviceInfo, JsonObject } from "../types";
import { useWorkspaceDraft } from "../workspaceDraft";
import { flowConnectorInfo, interactionReadAccount } from "../api";

const EMPTY: Record<OperatorRecordKind, JsonObject> = {
  account: {
    username: "",
    platform: "tiktok",
    group: "",
    deviceIds: [],
    credentialRef: "",
    notes: "",
  },
  network: {
    protocol: "http",
    host: "",
    port: 8080,
    username: "",
    credentialRef: "",
    deviceIds: [],
  },
  savedTask: {
    appId: "",
    appRevision: 1,
    target: { type: "explicit", udids: [] },
    inputs: {},
  },
};
export function OperatorRecordsPage({
  kind,
  devices,
}: {
  kind: OperatorRecordKind;
  devices: DeviceInfo[];
}) {
  const [records, setRecords] = useState<OperatorRecord[]>([]),
    [search, setSearch] = useState("");
  const [draft, setDraft] = useState<{
    id: string;
    name: string;
    expectedRevision: number | null;
    data: JsonObject;
  } | null>(null);
  const [baseline, setBaseline] = useState(""),
    [error, setError] = useState<string | null>(null),
    [busy, setBusy] = useState(false);
  const [result, setResult] = useState("");
  const [secrets, setSecrets] = useState<string[]>([]);
  const load = useCallback(async () => {
    try {
      setRecords(await operatorList(kind));
      setError(null);
    } catch (cause) {
      setError(describeError(cause));
    }
  }, [kind]);
  useEffect(() => {
    void load();
    void flowConnectorInfo()
      .then((info) => setSecrets(info.credentialNames))
      .catch(() => {});
  }, [load]);
  const dirty = draft !== null && JSON.stringify(draft) !== baseline;
  const save = async () => {
    if (!draft || busy) return false;
    setBusy(true);
    setError(null);
    try {
      await operatorSave({ ...draft, kind });
      setDraft(null);
      await load();
      return true;
    } catch (cause) {
      setError(describeError(cause));
      return false;
    } finally {
      setBusy(false);
    }
  };
  useWorkspaceDraft({
    id: `operator-${kind}`,
    label: kind === "account" ? "Tài khoản" : "Mạng",
    dirty,
    snapshotKey: JSON.stringify(draft),
    save,
    discard: () => setDraft(null),
  });
  const edit = (record?: OperatorRecord) => {
    const value = record
      ? {
          id: record.id,
          name: record.name,
          expectedRevision: record.revision,
          data: structuredClone(record.data),
        }
      : {
          id: crypto.randomUUID(),
          name: "",
          expectedRevision: null,
          data: structuredClone(EMPTY[kind]),
        };
    setDraft(value);
    setBaseline(JSON.stringify(value));
  };
  const field = (key: string, value: string | number | string[]) =>
    setDraft(
      (current) =>
        current && { ...current, data: { ...current.data, [key]: value } },
    );
  const exportRows = () => {
    const body = JSON.stringify(
      {
        schemaVersion: 1,
        kind,
        records: records.map(({ name, data }) => ({ name, data })),
      },
      null,
      2,
    );
    const url = URL.createObjectURL(
      new Blob([body], { type: "application/json" }),
    );
    const anchor = document.createElement("a");
    anchor.href = url;
    anchor.download = `riviu-${kind}.json`;
    anchor.click();
    URL.revokeObjectURL(url);
  };
  const visible = records.filter((record) =>
    `${record.name} ${record.data.username ?? ""} ${record.data.group ?? ""}`
      .toLowerCase()
      .includes(search.toLowerCase()),
  );
  return (
    <section className="operator-records">
      <div className="operator-toolbar">
        <input
          type="search"
          aria-label="Tìm bản ghi"
          placeholder="Tìm tên, tài khoản, nhóm…"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
        />
        <span>{visible.length} bản ghi</span>
        <button type="button" onClick={() => void load()}>
          <RefreshCw size={16} />
        </button>
        <button type="button" onClick={exportRows}>
          <Download size={16} />
          Xuất
        </button>
        <label className="operator-import">Nhập JSON<input type="file" accept=".json" onChange={async event=>{const file=event.target.files?.[0];if(!file)return;setBusy(true);try{if(file.size>4_000_000)throw new Error("Tệp vượt quá 4 MB");const payload=JSON.parse(await file.text()) as {schemaVersion:number;kind:OperatorRecordKind;records:{name:string;data:JsonObject}[]};if(payload.schemaVersion!==1||payload.kind!==kind||!Array.isArray(payload.records))throw new Error("Tệp không đúng loại dữ liệu");const imported=await operatorImport(kind,payload.records);setResult(`Đã nhập ${imported.length} bản ghi`);await load();}catch(cause){setError(describeError(cause));}finally{setBusy(false);event.target.value="";}}}/></label>
        <button className="primary" type="button" onClick={() => edit()}>
          <Plus size={16} />
          Thêm {kind === "account" ? "tài khoản" : "kết nối"}
        </button>
      </div>
      {error && (
        <p role="alert" className="control-stream-error">
          {error}
        </p>
      )}
      {result && <p role="status">{result}</p>}
      <table>
        <thead>
          <tr>
            <th>Tên</th>
            <th>{kind === "account" ? "Tài khoản" : "Máy chủ"}</th>
            <th>{kind === "account" ? "Nền tảng" : "Giao thức"}</th>
            <th>Thiết bị</th>
            <th>Thao tác</th>
          </tr>
        </thead>
        <tbody>
          {visible.map((record) => (
            <tr key={record.id}>
              <td>{record.name}</td>
              <td>{String(record.data.username || record.data.host || "—")}</td>
              <td>
                {String(record.data.platform || record.data.protocol || "—")}
              </td>
              <td>
                {Array.isArray(record.data.deviceIds)
                  ? record.data.deviceIds.length
                  : 0}
              </td>
              <td>
                {kind === "network" && (
                  <>
                    <button
                      type="button"
                      disabled={busy}
                      onClick={async () => {
                        setBusy(true);
                        try {
                          const checked = await operatorNetworkProbe(record.id);
                          setResult(
                            `TCP: ${checked.host}:${checked.port} - ${checked.elapsedMs} ms`,
                          );
                          setError(null);
                        } catch (cause) {
                          setError(describeError(cause));
                        } finally {
                          setBusy(false);
                        }
                      }}
                    >
                      TCP
                    </button>
                    {[false, true].map((clear) => (
                      <button
                        key={String(clear)}
                        type="button"
                        disabled={busy}
                        onClick={async () => {
                          if (
                            !(await requestConfirm({
                              title: clear ? "Xóa proxy trên máy?" : "Áp dụng proxy trên máy?",
                              message: record.name,
                              confirmLabel: "OK",
                            }))
                          )
                            return;
                          setBusy(true);
                          try {
                            const results = await operatorNetworkApply(
                              record.id,
                              clear,
                            );
                            setResult(
                              results
                                .map(
                                  (row) =>
                                    `${row.udid}: ${row.confirmed ? row.observed : JSON.stringify(row.error)}`,
                                )
                                .join("; "),
                            );
                            setError(null);
                          } catch (cause) {
                            setError(describeError(cause));
                          } finally {
                            setBusy(false);
                          }
                        }}
                      >
                        {clear ? "Xóa proxy" : "Áp dụng"}
                      </button>
                    ))}
                  </>
                )}
                {kind === "account" && (
                  <button
                    type="button"
                    disabled={busy}
                    onClick={async () => {
                      setBusy(true);
                      try {
                        const ids = Array.isArray(record.data.deviceIds)
                          ? record.data.deviceIds.filter(
                              (v): v is string => typeof v === "string",
                            )
                          : [];
                        const readings = await Promise.allSettled(
                          ids.map((id) => interactionReadAccount(id)),
                        );
                        setResult(
                          readings
                            .map((reading, i) =>
                              reading.status === "fulfilled"
                                ? `${ids[i]}: ${reading.value.observedHandle ?? reading.value.status}`
                                : `${ids[i]}: ${describeError(reading.reason)}`,
                            )
                            .join("; ") || "Chọn thiết bị trước khi đọc tài khoản",
                        );
                      } finally {
                        setBusy(false);
                      }
                    }}
                  >
                    Đọc tài khoản
                  </button>
                )}
                <button type="button" onClick={() => edit(record)}>
                  Chỉnh sửa
                </button>
                <button
                  type="button"
                  aria-label={`Lưu trữ ${record.name}`}
                  onClick={async () => {
                    if (
                      await requestConfirm({
                        title: "Lưu trữ bản ghi?",
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
      {!visible.length && (
        <p className="operator-empty">
          Chưa có bản ghi. Bấm Thêm để tạo và gắn thiết bị.
        </p>
      )}
      {draft && (
        <aside className="operator-record-editor" aria-label="Chỉnh bản ghi">
          <header>
            <strong>{draft.expectedRevision ? "Chỉnh sửa" : "Thêm mới"}</strong>
            <button
              type="button"
              aria-label="Đóng bản ghi"
              onClick={async () => {
                if (
                  !dirty ||
                  (await requestConfirm({
                    title: "Bỏ thay đổi?",
                    message: "Bản ghi chưa lưu.",
                    confirmLabel: "Bỏ thay đổi",
                  }))
                )
                  setDraft(null);
              }}
            >
              <X size={17} />
            </button>
          </header>
          <label>
            Tên
            <input
              value={draft.name}
              onChange={(e) => setDraft({ ...draft, name: e.target.value })}
            />
          </label>
          {kind === "account" ? (
            <>
              <label>
                Tài khoản
                <input
                  value={String(draft.data.username)}
                  onChange={(e) => field("username", e.target.value)}
                />
              </label>
              <label>
                Nền tảng
                <select
                  value={String(draft.data.platform)}
                  onChange={(e) => field("platform", e.target.value)}
                >
                  {["tiktok", "instagram", "threads", "other"].map((v) => (
                    <option key={v}>{v}</option>
                  ))}
                </select>
              </label>
              <label>
                Nhóm
                <input
                  value={String(draft.data.group ?? "")}
                  onChange={(e) => field("group", e.target.value)}
                />
              </label>
            </>
          ) : (
            <>
              <label>
                Giao thức
                <select
                  value={String(draft.data.protocol)}
                  onChange={(e) => field("protocol", e.target.value)}
                >
                  {["http", "https", "socks5", "router"].map((v) => (
                    <option key={v}>{v}</option>
                  ))}
                </select>
              </label>
              <label>
                Máy chủ
                <input
                  value={String(draft.data.host)}
                  onChange={(e) => field("host", e.target.value)}
                />
              </label>
              <label>
                Cổng
                <input
                  type="number"
                  min={1}
                  max={65535}
                  value={Number(draft.data.port)}
                  onChange={(e) => field("port", Number(e.target.value))}
                />
              </label>
            </>
          )}
          <label>
            Thông tin xác thực
            <select
              value={String(draft.data.credentialRef ?? "")}
              onChange={(e) => field("credentialRef", e.target.value)}
            >
              <option value="">Không dùng</option>
              {secrets.map((s) => (
                <option key={s}>{s}</option>
              ))}
            </select>
          </label>
          <fieldset>
            <legend>Thiết bị</legend>
            {devices.map((device) => (
              <label key={device.udid}>
                <input
                  type="checkbox"
                  checked={
                    Array.isArray(draft.data.deviceIds) &&
                    draft.data.deviceIds.includes(device.udid)
                  }
                  onChange={(e) => {
                    const ids = Array.isArray(draft.data.deviceIds)
                      ? draft.data.deviceIds.filter(
                          (v): v is string => typeof v === "string",
                        )
                      : [];
                    field(
                      "deviceIds",
                      e.target.checked
                        ? [...ids, device.udid]
                        : ids.filter((id) => id !== device.udid),
                    );
                  }}
                />
                {device.name}
              </label>
            ))}
          </fieldset>
          <button
            type="button"
            className="primary"
            disabled={busy || !draft.name.trim()}
            onClick={() => void save()}
          >
            <Save size={16} />
            Lưu bản ghi
          </button>
        </aside>
      )}
    </section>
  );
}
