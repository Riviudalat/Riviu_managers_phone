import { useEffect, useState } from "react";
import {
  addAppLibrary,
  addMaterial,
  analyticsSummary,
  apiDocs,
  authLogin,
  authRegister,
  createPublishTask,
  deleteAppLibrary,
  deleteGroup,
  deleteMaterial,
  deleteProxy,
  deleteSchedule,
  exampleScript,
  exportProxyConfig,
  installLibraryApp,
  listAppsLibrary,
  listGroups,
  listMaterials,
  listOpLogs,
  listProxies,
  listPublishTasks,
  listSchedules,
  listScripts,
  listUsers,
  pushMaterial,
  saveGroup,
  saveProxy,
  saveSchedule,
  saveScript,
} from "../api";
import { SelectionStrip, flash, targetsOf } from "../components/SelectionStrip";
import { pickIpa, pickMaterial } from "../pickFile";
import type {
  AnalyticsSummary,
  AppLibraryItem,
  DeviceGroup,
  DeviceInfo,
  LocalUser,
  MaterialItem,
  OpLog,
  ProxyConfig,
  PublishTask,
  ScheduleItem,
} from "../types";

type SelProps = {
  devices: DeviceInfo[];
  selected: string[];
  onSelectUdids: (udids: string[]) => void;
};

export function GroupsPage({ devices, selected, onSelectUdids }: SelProps) {
  const [groups, setGroups] = useState<DeviceGroup[]>([]);
  const [name, setName] = useState("");
  const [msg, setMsg] = useState<string | null>(null);
  const targets = targetsOf(selected, devices);

  const reload = () =>
    listGroups()
      .then(setGroups)
      .catch((e) => setMsg(String(e)));
  useEffect(() => {
    reload();
  }, []);

  return (
    <div className="panel">
      <header className="panel-header">
        <h2>Nhóm thiết bị</h2>
        <button type="button" className="ghost" onClick={reload}>
          Refresh
        </button>
      </header>
      <SelectionStrip
        devices={devices}
        selected={selected}
        onSelectAll={() => onSelectUdids(devices.map((d) => d.udid))}
        onClear={() => onSelectUdids([])}
      />
      <div className="panel-grid">
        <section>
          <h3>Tạo nhóm</h3>
          <label>
            Tên nhóm
            <input value={name} onChange={(e) => setName(e.target.value)} placeholder="Farm A" />
          </label>
          <button
            type="button"
            className="primary"
            disabled={!name.trim() || !targets.length}
            onClick={async () => {
              try {
                await saveGroup({
                  id: "",
                  name: name.trim(),
                  color: "#FF6A00",
                  udids: targets,
                  createdAt: "",
                });
                setName("");
                setMsg(null);
                await reload();
                flash(`Đã lưu nhóm «${name.trim()}» · ${targets.length} máy`);
              } catch (e) {
                setMsg(String(e));
              }
            }}
          >
            Lưu nhóm ({targets.length})
          </button>
          {msg && <p className="error">{msg}</p>}
        </section>
        <section>
          <h3>Danh sách</h3>
          <div className="job-list">
            {groups.map((g) => (
              <article key={g.id} className="job-card">
                <div>
                  <strong style={{ color: g.color }}>{g.name}</strong>
                  <span className="pill">{g.udids.length} devices</span>
                </div>
                <p className="hint mono">{g.udids.join(", ") || "—"}</p>
                <div className="row">
                  <button type="button" className="primary" onClick={() => onSelectUdids(g.udids)}>
                    Chọn nhóm
                  </button>
                  <button
                    type="button"
                    className="ghost"
                    onClick={async () => {
                      await deleteGroup(g.id);
                      await reload();
                    }}
                  >
                    Xóa
                  </button>
                </div>
              </article>
            ))}
            {!groups.length && <p className="hint">Chưa có nhóm — tạo ở cột trái</p>}
          </div>
        </section>
      </div>
    </div>
  );
}

export function ProxyPage() {
  const [items, setItems] = useState<ProxyConfig[]>([]);
  const [msg, setMsg] = useState<string | null>(null);
  const [form, setForm] = useState<ProxyConfig>({
    id: "",
    name: "",
    proxyType: "http",
    host: "",
    port: 8080,
    username: "",
    password: "",
    notes: "",
  });

  const reload = () => listProxies().then(setItems).catch((e) => setMsg(String(e)));
  useEffect(() => {
    reload();
  }, []);

  return (
    <div className="panel">
      <header className="panel-header">
        <h2>Proxy (config — không shop)</h2>
        <button type="button" className="ghost" onClick={reload}>
          Refresh
        </button>
      </header>
      <p className="hint">
        Lưu cấu hình local + Export copy. App không mua proxy / không ép traffic iPhone — bạn áp dụng
        thủ công (Wi‑Fi proxy / VPN).
      </p>
      <div className="panel-grid">
        <section>
          <h3>Thêm / sửa</h3>
          {(
            [
              ["name", "Tên"],
              ["proxyType", "Loại (http/socks5)"],
              ["host", "Host"],
              ["username", "User"],
              ["password", "Pass"],
              ["notes", "Ghi chú"],
            ] as const
          ).map(([key, label]) => (
            <label key={key}>
              {label}
              <input
                value={String(form[key] ?? "")}
                onChange={(e) => setForm({ ...form, [key]: e.target.value })}
              />
            </label>
          ))}
          <label>
            Port
            <input
              type="number"
              value={form.port}
              onChange={(e) => setForm({ ...form, port: Number(e.target.value) || 0 })}
            />
          </label>
          <button
            type="button"
            className="primary"
            disabled={!form.name.trim() || !form.host.trim()}
            onClick={async () => {
              try {
                await saveProxy(form);
                setForm({
                  id: "",
                  name: "",
                  proxyType: "http",
                  host: "",
                  port: 8080,
                  username: "",
                  password: "",
                  notes: "",
                });
                await reload();
                flash("Đã lưu proxy");
              } catch (e) {
                setMsg(String(e));
              }
            }}
          >
            Lưu proxy
          </button>
          {msg && <p className="error">{msg}</p>}
        </section>
        <section>
          <h3>Danh sách</h3>
          <div className="job-list">
            {items.map((p) => (
              <article key={p.id} className="job-card">
                <div>
                  <strong>{p.name}</strong>
                  <span className="pill">{p.proxyType}</span>
                </div>
                <p className="hint">
                  {p.host}:{p.port}
                </p>
                <div className="row">
                  <button type="button" className="ghost" onClick={() => setForm(p)}>
                    Sửa
                  </button>
                  <button
                    type="button"
                    className="ghost"
                    onClick={async () => {
                      try {
                        const text = await exportProxyConfig(p.id);
                        await navigator.clipboard.writeText(text);
                        flash("Đã copy config vào clipboard");
                      } catch (e) {
                        flash(String(e));
                      }
                    }}
                  >
                    Export
                  </button>
                  <button
                    type="button"
                    className="ghost"
                    onClick={async () => {
                      await deleteProxy(p.id);
                      await reload();
                    }}
                  >
                    Xóa
                  </button>
                </div>
              </article>
            ))}
            {!items.length && <p className="hint">Chưa có proxy</p>}
          </div>
        </section>
      </div>
    </div>
  );
}

export function MaterialPage({ devices, selected, onSelectUdids }: SelProps) {
  const [items, setItems] = useState<MaterialItem[]>([]);
  const [path, setPath] = useState("");
  const [busy, setBusy] = useState(false);
  const targets = targetsOf(selected, devices);
  const target = targets[0];

  const reload = () => listMaterials().then(setItems).catch((e) => flash(String(e)));
  useEffect(() => {
    reload();
  }, []);

  return (
    <div className="panel">
      <header className="panel-header">
        <h2>Material center</h2>
      </header>
      <SelectionStrip
        devices={devices}
        selected={selected}
        onSelectAll={() => onSelectUdids(devices.map((d) => d.udid))}
        onClear={() => onSelectUdids([])}
      />
      <div className="row" style={{ marginTop: 8 }}>
        <input
          style={{ flex: 1 }}
          value={path}
          onChange={(e) => setPath(e.target.value)}
          placeholder="Đường dẫn file…"
        />
        <button
          type="button"
          className="ghost"
          onClick={async () => {
            const p = await pickMaterial();
            if (p) setPath(p);
          }}
        >
          Chọn file…
        </button>
        <button
          type="button"
          className="primary"
          disabled={!path.trim() || busy}
          onClick={async () => {
            setBusy(true);
            try {
              await addMaterial(path.trim());
              setPath("");
              await reload();
              flash("Đã thêm material");
            } catch (e) {
              flash(String(e));
            } finally {
              setBusy(false);
            }
          }}
        >
          Thêm
        </button>
      </div>
      <div className="job-list" style={{ marginTop: 12 }}>
        {items.map((m) => (
          <article key={m.id} className="job-card">
            <div>
              <strong>{m.name}</strong>
              <span className="pill">{m.kind}</span>
            </div>
            <p className="hint">
              {(m.size / 1024).toFixed(1)} KB · {m.path}
            </p>
            <div className="row">
              <button
                type="button"
                className="primary"
                disabled={!target || busy}
                onClick={async () => {
                  setBusy(true);
                  try {
                    flash(await pushMaterial(target!, m.id));
                  } catch (e) {
                    flash(String(e));
                  } finally {
                    setBusy(false);
                  }
                }}
              >
                Push → {target ? target.slice(0, 8) : "?"}
              </button>
              <button
                type="button"
                className="ghost"
                onClick={async () => {
                  await deleteMaterial(m.id);
                  await reload();
                }}
              >
                Xóa
              </button>
            </div>
          </article>
        ))}
        {!items.length && <p className="hint">Chưa có material — bấm «Chọn file…»</p>}
      </div>
    </div>
  );
}

export function AppsPage({ devices, selected, onSelectUdids }: SelProps) {
  const [items, setItems] = useState<AppLibraryItem[]>([]);
  const [path, setPath] = useState("");
  const [bundleId, setBundleId] = useState("");
  const [busy, setBusy] = useState(false);
  const targets = targetsOf(selected, devices);

  const reload = () => listAppsLibrary().then(setItems).catch((e) => flash(String(e)));
  useEffect(() => {
    reload();
  }, []);

  return (
    <div className="panel">
      <header className="panel-header">
        <h2>App center</h2>
      </header>
      <SelectionStrip
        devices={devices}
        selected={selected}
        onSelectAll={() => onSelectUdids(devices.map((d) => d.udid))}
        onClear={() => onSelectUdids([])}
      />
      <div className="row" style={{ marginTop: 8 }}>
        <input
          style={{ flex: 1 }}
          value={path}
          onChange={(e) => setPath(e.target.value)}
          placeholder="Đường dẫn .ipa…"
        />
        <button
          type="button"
          className="ghost"
          onClick={async () => {
            const p = await pickIpa();
            if (p) setPath(p);
          }}
        >
          Chọn IPA…
        </button>
      </div>
      <label>
        Bundle ID (optional)
        <input value={bundleId} onChange={(e) => setBundleId(e.target.value)} />
      </label>
      <button
        type="button"
        className="primary"
        disabled={!path.trim() || busy}
        onClick={async () => {
          setBusy(true);
          try {
            await addAppLibrary(path.trim(), undefined, bundleId || undefined);
            setPath("");
            await reload();
            flash("Đã thêm IPA vào thư viện");
          } catch (e) {
            flash(String(e));
          } finally {
            setBusy(false);
          }
        }}
      >
        Thêm vào thư viện
      </button>
      <div className="job-list" style={{ marginTop: 12 }}>
        {items.map((a) => (
          <article key={a.id} className="job-card">
            <div>
              <strong>{a.name}</strong>
              <span className="pill">{a.bundleId || "no bundle"}</span>
            </div>
            <p className="hint">{a.path}</p>
            <div className="row">
              <button
                type="button"
                className="primary"
                disabled={!targets.length || busy}
                onClick={async () => {
                  setBusy(true);
                  try {
                    const errors: string[] = [];
                    for (const u of targets) {
                      try {
                        await installLibraryApp(u, a.id);
                      } catch (e) {
                        errors.push(`${u.slice(0, 8)}: ${e}`);
                      }
                    }
                    if (errors.length) flash(`Một số máy lỗi:\n${errors.join("\n")}`);
                    else flash(`Đã cài lên ${targets.length} máy`);
                  } finally {
                    setBusy(false);
                  }
                }}
              >
                Install → {targets.length} máy
              </button>
              <button
                type="button"
                className="ghost"
                onClick={async () => {
                  await deleteAppLibrary(a.id);
                  await reload();
                }}
              >
                Xóa
              </button>
            </div>
          </article>
        ))}
        {!items.length && <p className="hint">Chưa có IPA — bấm «Chọn IPA…»</p>}
      </div>
    </div>
  );
}

export function SyncPage({
  devices,
  selected,
  groupMode,
  onToggleGroup,
  onSelect,
  onSelectUdids,
}: {
  devices: DeviceInfo[];
  selected: string[];
  groupMode: boolean;
  onToggleGroup: () => void;
  onSelect: (udid: string, additive: boolean) => void;
  onSelectUdids: (udids: string[]) => void;
}) {
  const master = selected[0];
  const slaves = selected.slice(1);
  return (
    <div className="panel">
      <header className="panel-header">
        <h2>Đồng bộ cửa sổ</h2>
        <button type="button" className={groupMode ? "primary" : "ghost"} onClick={onToggleGroup}>
          Sync {groupMode ? "ON" : "OFF"}
        </button>
      </header>
      <SelectionStrip
        devices={devices}
        selected={selected}
        onSelectAll={() => onSelectUdids(devices.map((d) => d.udid))}
        onClear={() => onSelectUdids([])}
      />
      <p className="hint">
        Click máy để thêm/bớt selection. Máy đầu = Master. Bật Sync rồi điều khiển trong cửa sổ phóng
        to — input broadcast qua group_input.
      </p>
      <div className="job-list" style={{ marginTop: 12 }}>
        {devices.map((d) => (
          <article
            key={d.udid}
            className="job-card"
            style={{
              outline: selected.includes(d.udid) ? "2px solid var(--primary)" : undefined,
              cursor: "pointer",
            }}
            onClick={() => onSelect(d.udid, true)}
          >
            <div>
              <strong>{d.name}</strong>
              {d.udid === master && <span className="chip primary">Master</span>}
              {slaves.includes(d.udid) && <span className="chip ok">Slave</span>}
            </div>
            <p className="hint mono">{d.udid}</p>
          </article>
        ))}
        {!devices.length && <p className="hint">Chưa có thiết bị</p>}
      </div>
    </div>
  );
}

export function PublishPage({ devices, selected, onSelectUdids }: SelProps) {
  const [tasks, setTasks] = useState<PublishTask[]>([]);
  const [scripts, setScripts] = useState<[string, string][]>([]);
  const [materials, setMaterials] = useState<MaterialItem[]>([]);
  const [name, setName] = useState("publish-1");
  const [scriptName, setScriptName] = useState("");
  const [materialId, setMaterialId] = useState("");
  const [busy, setBusy] = useState(false);
  const targets = targetsOf(selected, devices);

  const reload = async () => {
    setTasks(await listPublishTasks());
    let scriptsList = await listScripts();
    if (!scriptsList.length) {
      const body = await exampleScript();
      await saveScript("example", body);
      scriptsList = await listScripts();
    }
    setScripts(scriptsList);
    if (!scriptName && scriptsList.length) setScriptName(scriptsList[0][0]);
    setMaterials(await listMaterials());
  };
  useEffect(() => {
    reload().catch((e) => flash(String(e)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div className="panel">
      <header className="panel-header">
        <h2>Publish tasks</h2>
      </header>
      <SelectionStrip
        devices={devices}
        selected={selected}
        onSelectAll={() => onSelectUdids(devices.map((d) => d.udid))}
        onClear={() => onSelectUdids([])}
      />
      <p className="hint">Chạy script automation local trên máy đã chọn (không gọi mạng xã hội cloud).</p>
      <label>
        Tên task
        <input value={name} onChange={(e) => setName(e.target.value)} />
      </label>
      <label>
        Script
        <select value={scriptName} onChange={(e) => setScriptName(e.target.value)}>
          <option value="">— chọn —</option>
          {scripts.map(([n]) => (
            <option key={n} value={n}>
              {n}
            </option>
          ))}
        </select>
      </label>
      <label>
        Material (optional)
        <select value={materialId} onChange={(e) => setMaterialId(e.target.value)}>
          <option value="">—</option>
          {materials.map((m) => (
            <option key={m.id} value={m.id}>
              {m.name}
            </option>
          ))}
        </select>
      </label>
      <button
        type="button"
        className="primary"
        disabled={!scriptName || !targets.length || busy}
        onClick={async () => {
          setBusy(true);
          try {
            const t = await createPublishTask(
              name,
              scriptName,
              materialId ? [materialId] : [],
              targets,
            );
            await reload();
            flash(`Task «${t.name}» → ${t.status} · ${targets.length} máy`);
          } catch (e) {
            flash(String(e));
          } finally {
            setBusy(false);
          }
        }}
      >
        Tạo &amp; chạy ({targets.length} máy)
      </button>
      <div className="job-list" style={{ marginTop: 12 }}>
        {tasks.map((t) => (
          <article key={t.id} className="job-card">
            <div>
              <strong>{t.name}</strong>
              <span className={`pill ${t.status}`}>{t.status}</span>
            </div>
            <p className="hint">
              {t.scriptName} · {t.udids.length} devices · {new Date(t.createdAt).toLocaleString()}
            </p>
          </article>
        ))}
        {!tasks.length && <p className="hint">Chưa có publish task</p>}
      </div>
    </div>
  );
}

export function DataPage() {
  const [data, setData] = useState<AnalyticsSummary | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const load = () =>
    analyticsSummary()
      .then((d) => {
        setData(d);
        setErr(null);
      })
      .catch((e) => setErr(String(e)));
  useEffect(() => {
    load();
  }, []);
  if (err) return <div className="panel error">{err}</div>;
  if (!data) return <div className="panel">Loading…</div>;
  return (
    <div className="panel">
      <header className="panel-header">
        <h2>Data center</h2>
        <button type="button" className="ghost" onClick={load}>
          Refresh
        </button>
      </header>
      <div className="stats-grid">
        {(
          [
            ["Devices", `${data.deviceReady}/${data.deviceTotal}`],
            ["Jobs ok", String(data.jobsSucceeded)],
            ["Jobs fail", String(data.jobsFailed)],
            ["Running", String(data.jobsRunning)],
            ["Scripts", String(data.scriptsTotal)],
            ["Materials", String(data.materialsTotal)],
            ["Apps", String(data.appsTotal)],
            ["Schedules", String(data.schedulesEnabled)],
          ] as const
        ).map(([k, v]) => (
          <article key={k} className="job-card">
            <div className="hint">{k}</div>
            <strong style={{ fontSize: "1.4rem" }}>{v}</strong>
          </article>
        ))}
      </div>
    </div>
  );
}

export function TeamPage() {
  const [users, setUsers] = useState<LocalUser[]>([]);
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [msg, setMsg] = useState<string | null>(null);

  const reload = () => listUsers().then(setUsers).catch((e) => setMsg(String(e)));
  useEffect(() => {
    reload();
  }, []);

  return (
    <div className="panel">
      <header className="panel-header">
        <h2>Thành viên (local)</h2>
        <button type="button" className="ghost" onClick={reload}>
          Refresh
        </button>
      </header>
      <p className="hint">Tài khoản local trên máy này — không đồng bộ cloud.</p>
      <div className="panel-grid">
        <section>
          <h3>Thêm thành viên</h3>
          <label>
            Email
            <input value={email} onChange={(e) => setEmail(e.target.value)} />
          </label>
          <label>
            Password
            <input
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
            />
          </label>
          <button
            type="button"
            className="primary"
            disabled={!email.trim() || password.length < 4}
            onClick={async () => {
              try {
                await authRegister(email.trim(), password);
                setEmail("");
                setPassword("");
                setMsg(null);
                await reload();
                flash("Đã tạo user");
              } catch (e) {
                setMsg(String(e));
              }
            }}
          >
            Tạo user
          </button>
          {msg && <p className="error">{msg}</p>}
        </section>
        <section>
          <h3>Danh sách</h3>
          <div className="job-list">
            {users.map((u) => (
              <article key={u.id} className="job-card">
                <div>
                  <strong>{u.email}</strong>
                  <span className="pill">{u.role}</span>
                </div>
                <p className="hint">{u.createdAt}</p>
              </article>
            ))}
            {!users.length && <p className="hint">Chưa có user (guest vẫn dùng được app)</p>}
          </div>
        </section>
      </div>
    </div>
  );
}

export function LogsPage() {
  const [logs, setLogs] = useState<OpLog[]>([]);
  const load = () => listOpLogs(200).then(setLogs).catch((e) => flash(String(e)));
  useEffect(() => {
    load();
  }, []);
  return (
    <div className="panel">
      <header className="panel-header">
        <h2>Operation logs</h2>
        <button type="button" className="ghost" onClick={load}>
          Refresh
        </button>
      </header>
      <div className="job-list">
        {logs.map((l) => (
          <article key={l.id} className="job-card">
            <div>
              <strong>{l.action}</strong>
              <span className="hint">{new Date(l.createdAt).toLocaleString()}</span>
            </div>
            <p className="hint">{l.detail}</p>
          </article>
        ))}
        {!logs.length && <p className="hint">Chưa có log — thao tác Start/Agent/Publish sẽ ghi ở đây</p>}
      </div>
    </div>
  );
}

export function AccountPage({
  user,
  onShowAuth,
}: {
  user: LocalUser | null;
  onShowAuth: () => void;
}) {
  return (
    <div className="panel">
      <header className="panel-header">
        <h2>Account settings</h2>
      </header>
      <section className="settings-card">
        <h3>Session</h3>
        <p>
          User: <code>{user?.email ?? "guest@local"}</code> · role{" "}
          <code>{user?.role ?? "admin"}</code>
        </p>
        <p className="hint">
          Login/Register ẩn mặc định. Set <code>RIVIU_SHOW_AUTH=1</code> rồi restart, hoặc:
        </p>
        <button type="button" className="ghost" onClick={onShowAuth}>
          Hiện màn Login (dev)
        </button>
      </section>
    </div>
  );
}

export function ApiPage() {
  const [docs, setDocs] = useState("Loading…");
  useEffect(() => {
    apiDocs()
      .then(setDocs)
      .catch((e) => setDocs(String(e)));
  }, []);
  return (
    <div className="panel">
      <header className="panel-header">
        <h2>API</h2>
      </header>
      <pre className="api-docs">{docs}</pre>
    </div>
  );
}

export function ScheduleBlock({
  devices,
  selected,
  onSelectUdids,
}: SelProps) {
  const [items, setItems] = useState<ScheduleItem[]>([]);
  const [scripts, setScripts] = useState<[string, string][]>([]);
  const [name, setName] = useState("hourly");
  const [scriptName, setScriptName] = useState("");
  const [mins, setMins] = useState(60);
  const targets = targetsOf(selected, devices);

  const reload = async () => {
    setItems(await listSchedules());
    let scriptsList = await listScripts();
    if (!scriptsList.length) {
      const body = await exampleScript();
      await saveScript("example", body);
      scriptsList = await listScripts();
    }
    setScripts(scriptsList);
    if (!scriptName && scriptsList.length) setScriptName(scriptsList[0][0]);
  };
  useEffect(() => {
    reload().catch((e) => flash(String(e)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <section style={{ marginTop: 16 }}>
      <h3>Task schedule</h3>
      <SelectionStrip
        devices={devices}
        selected={selected}
        onSelectAll={() => onSelectUdids(devices.map((d) => d.udid))}
        onClear={() => onSelectUdids([])}
      />
      <label>
        Tên
        <input value={name} onChange={(e) => setName(e.target.value)} />
      </label>
      <label>
        Script
        <select value={scriptName} onChange={(e) => setScriptName(e.target.value)}>
          <option value="">—</option>
          {scripts.map(([n]) => (
            <option key={n} value={n}>
              {n}
            </option>
          ))}
        </select>
      </label>
      <label>
        Mỗi (phút)
        <input
          type="number"
          value={mins}
          onChange={(e) => setMins(Number(e.target.value) || 60)}
        />
      </label>
      <button
        type="button"
        className="primary"
        disabled={!scriptName || !targets.length}
        onClick={async () => {
          try {
            await saveSchedule({
              id: "",
              name,
              scriptName,
              udids: targets,
              everyMinutes: mins,
              enabled: true,
            });
            await reload();
            flash(`Schedule «${name}» mỗi ${mins} phút · ${targets.length} máy`);
          } catch (e) {
            flash(String(e));
          }
        }}
      >
        Lưu schedule ({targets.length})
      </button>
      <div className="job-list" style={{ marginTop: 8 }}>
        {items.map((s) => (
          <article key={s.id} className="job-card">
            <div>
              <strong>{s.name}</strong>
              <span className="pill">{s.enabled ? "on" : "off"}</span>
            </div>
            <p className="hint">
              {s.scriptName} · every {s.everyMinutes}m · next {s.nextRunAt ?? "—"}
            </p>
            <button
              type="button"
              className="ghost"
              onClick={async () => {
                await deleteSchedule(s.id);
                await reload();
              }}
            >
              Xóa
            </button>
          </article>
        ))}
      </div>
    </section>
  );
}

export function LoginPage({
  onDone,
  onRegister,
}: {
  onDone: (u: LocalUser) => void;
  onRegister: () => void;
}) {
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [err, setErr] = useState<string | null>(null);
  return (
    <div className="panel" style={{ maxWidth: 420, margin: "3rem auto" }}>
      <header className="panel-header">
        <h2>Login</h2>
      </header>
      <label>
        Email
        <input value={email} onChange={(e) => setEmail(e.target.value)} />
      </label>
      <label>
        Password
        <input
          type="password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
        />
      </label>
      {err && <p className="error">{err}</p>}
      <div className="row">
        <button
          type="button"
          className="primary"
          onClick={async () => {
            try {
              onDone(await authLogin(email, password));
            } catch (e) {
              setErr(String(e));
            }
          }}
        >
          Đăng nhập
        </button>
        <button type="button" className="linkish" onClick={onRegister}>
          Đăng ký
        </button>
      </div>
    </div>
  );
}

export function RegisterPage({
  onDone,
  onLogin,
}: {
  onDone: (u: LocalUser) => void;
  onLogin: () => void;
}) {
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [err, setErr] = useState<string | null>(null);
  return (
    <div className="panel" style={{ maxWidth: 420, margin: "3rem auto" }}>
      <header className="panel-header">
        <h2>Register</h2>
      </header>
      <label>
        Email
        <input value={email} onChange={(e) => setEmail(e.target.value)} />
      </label>
      <label>
        Password
        <input
          type="password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
        />
      </label>
      {err && <p className="error">{err}</p>}
      <div className="row">
        <button
          type="button"
          className="primary"
          onClick={async () => {
            try {
              onDone(await authRegister(email, password));
            } catch (e) {
              setErr(String(e));
            }
          }}
        >
          Tạo tài khoản
        </button>
        <button type="button" className="linkish" onClick={onLogin}>
          Đã có tài khoản
        </button>
      </div>
    </div>
  );
}
