import { useEffect, useState } from "react";
import {
  addAppLibrary,
  addMaterial,
  analyticsSummary,
  apiDocs,
  authLogin,
  authRegister,
  deleteAppLibrary,
  deleteGroup,
  deleteMaterial,
  deleteProxy,
  deleteSchedule,
  exampleScript,
  exportProxyConfig,
  installIpaToGroup,
  installLibraryApp,
  listAppsLibrary,
  listGroups,
  listMaterials,
  listOpLogs,
  listProxies,
  listSchedules,
  listScripts,
  listUsers,
  publishCancel,
  publishCreateCampaign,
  publishList,
  publishPrepare,
  publishPost,
  publishScanFolder,
  publishTransfer,
  pushMaterial,
  saveGroup,
  saveProxy,
  saveSchedule,
  saveScript,
} from "../api";
import { SelectionStrip, flash, flashError, targetsOf } from "../components/SelectionStrip";
import { EmptyState } from "../components/States";
import {
  IconApp,
  IconImage,
  IconLog,
  IconPhone,
  IconProxy,
  IconRocket,
  IconUser,
  IconUsers,
} from "../components/Icons";
import { pickDirectory, pickIpa, pickMaterial } from "../pickFile";
import type {
  AnalyticsSummary,
  AppLibraryItem,
  DeviceGroup,
  DeviceInfo,
  GroupInstallResult,
  LocalUser,
  MaterialItem,
  OpLog,
  ProxyConfig,
  PublishBundle,
  PublishCampaignRecord,
  PublishFolderManifest,
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
          Làm mới
        </button>
      </header>
      <SelectionStrip
        devices={devices}
        selected={selected}
        onSelectAll={() => onSelectUdids(devices.map((d) => d.udid))}
        onClear={() => onSelectUdids([])}
        onSelectUdids={onSelectUdids}
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
            {!groups.length && (
              <EmptyState
                compact
                icon={<IconUsers size={15} />}
                title="Chưa có nhóm nào"
                hint="Tạo nhóm ở cột bên trái để chạy hàng loạt trên nhiều máy."
              />
            )}
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
          Làm mới
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
                        flashError(e);
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
            {!items.length && (
              <EmptyState
                compact
                icon={<IconProxy size={15} />}
                title="Chưa có proxy"
                hint="Thêm proxy ở khung bên trên để gán cho thiết bị."
              />
            )}
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

  const reload = () => listMaterials().then(setItems).catch((e) => flashError(e));
  useEffect(() => {
    reload();
  }, []);

  return (
    <div className="panel">
      <header className="panel-header">
        <h2>Kho nội dung</h2>
      </header>
      <SelectionStrip
        devices={devices}
        selected={selected}
        onSelectAll={() => onSelectUdids(devices.map((d) => d.udid))}
        onClear={() => onSelectUdids([])}
        onSelectUdids={onSelectUdids}
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
              flashError(e);
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
                    flashError(e);
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
        {!items.length && (
          <EmptyState
            compact
            icon={<IconImage size={15} />}
            title="Chưa có nội dung"
            hint="Bấm «Chọn file…» để thêm ảnh hoặc video vào kho."
          />
        )}
      </div>
    </div>
  );
}

export function AppsPage({ devices, selected, onSelectUdids }: SelProps) {
  const [items, setItems] = useState<AppLibraryItem[]>([]);
  const [path, setPath] = useState("");
  const [bundleId, setBundleId] = useState("");
  const [busy, setBusy] = useState(false);
  const [groups, setGroups] = useState<DeviceGroup[]>([]);
  const [groupId, setGroupId] = useState("");
  const [groupResults, setGroupResults] = useState<GroupInstallResult[]>([]);
  const targets = targetsOf(selected, devices);

  const reload = () => listAppsLibrary().then(setItems).catch((e) => flashError(e));
  useEffect(() => {
    reload();
    listGroups().then(setGroups).catch((e) => flashError(e));
  }, []);

  const installToGroup = async (ipaPath: string) => {
    if (!groupId) {
      flash("Chọn một nhóm trước");
      return;
    }
    setBusy(true);
    setGroupResults([]);
    try {
      const results = await installIpaToGroup(groupId, ipaPath);
      setGroupResults(results);
      const failed = results.filter((r) => !r.ok).length;
      flash(
        failed
          ? `Cài xong: ${results.length - failed} OK, ${failed} lỗi`
          : `Đã cài lên ${results.length} máy trong nhóm`,
      );
    } catch (e) {
      flashError(e);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="panel">
      <header className="panel-header">
        <h2>Trung tâm ứng dụng</h2>
      </header>
      <SelectionStrip
        devices={devices}
        selected={selected}
        onSelectAll={() => onSelectUdids(devices.map((d) => d.udid))}
        onClear={() => onSelectUdids([])}
        onSelectUdids={onSelectUdids}
      />
      <div className="row" style={{ marginTop: 8 }}>
        <label style={{ flex: 1 }}>
          Cài hàng loạt theo nhóm
          <select value={groupId} onChange={(e) => setGroupId(e.target.value)}>
            <option value="">— chọn nhóm —</option>
            {groups.map((g) => (
              <option key={g.id} value={g.id}>
                {g.name} ({g.udids.length} máy)
              </option>
            ))}
          </select>
        </label>
      </div>
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
            flashError(e);
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
                className="primary"
                disabled={!groupId || busy}
                title="Cài lên toàn bộ máy trong nhóm đã chọn (chạy phía backend)"
                onClick={() => installToGroup(a.path)}
              >
                Cài → nhóm
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
        {!items.length && (
          <EmptyState
            compact
            icon={<IconApp size={15} />}
            title="Chưa có IPA"
            hint="Bấm «Chọn IPA…» để thêm ứng dụng vào thư viện."
          />
        )}
      </div>
      {groupResults.length > 0 && (
        <div className="job-list" style={{ marginTop: 12 }}>
          <h4>Kết quả cài theo nhóm</h4>
          {groupResults.map((r) => (
            <article key={r.udid} className="job-card">
              <div>
                <span className="pill">{r.ok ? "✅ OK" : "❌ Lỗi"}</span>
                <span className="mono">{r.udid.slice(0, 12)}</span>
              </div>
              {r.error && <p className="hint">{r.error}</p>}
            </article>
          ))}
        </div>
      )}
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
        onSelectUdids={onSelectUdids}
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
              {d.udid === master && <span className="chip primary">Máy chính</span>}
              {slaves.includes(d.udid) && <span className="chip ok">Máy phụ</span>}
            </div>
            <p className="hint mono">{d.udid}</p>
          </article>
        ))}
        {!devices.length && (
          <EmptyState
            compact
            icon={<IconPhone size={15} />}
            title="Chưa có thiết bị"
            hint="Về Quản lý cửa sổ và làm mới danh sách."
          />
        )}
      </div>
    </div>
  );
}

export function PublishPage({ devices, selected, onSelectUdids }: SelProps) {
  const [sourceRoot, setSourceRoot] = useState("");
  const [manifest, setManifest] = useState<PublishFolderManifest | null>(null);
  const [bundleIds, setBundleIds] = useState<string[]>([]);
  const [runAt, setRunAt] = useState("");
  const [campaigns, setCampaigns] = useState<PublishCampaignRecord[]>([]);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);
  const targets = targetsOf(selected, devices);

  const reload = () => publishList().then(setCampaigns).catch((e) => setMsg(String(e)));
  useEffect(() => {
    reload();
  }, []);

  const selectedBundles =
    manifest?.bundles.filter((bundle) => bundleIds.includes(bundle.id)) ?? [];
  const mappingReady = selectedBundles.length > 0 && selectedBundles.length === targets.length;

  const scan = async (path: string) => {
    setBusy(true);
    setMsg(null);
    try {
      const next = await publishScanFolder(path);
      setSourceRoot(path);
      setManifest(next);
      setBundleIds(next.bundles.slice(0, targets.length).map((bundle) => bundle.id));
    } catch (e) {
      setManifest(null);
      setBundleIds([]);
      setMsg(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="panel">
      <header className="panel-header">
        <h2>Đăng carousel</h2>
        <button type="button" className="ghost" onClick={reload} disabled={busy}>
          Làm mới
        </button>
      </header>
      <SelectionStrip
        devices={devices}
        selected={selected}
        onSelectAll={() => onSelectUdids(devices.map((d) => d.udid))}
        onClear={() => onSelectUdids([])}
        onSelectUdids={onSelectUdids}
      />
      <div className="row" style={{ marginTop: 8 }}>
        <input
          style={{ flex: 1 }}
          value={sourceRoot}
          onChange={(e) => setSourceRoot(e.target.value)}
          placeholder="Thư mục chứa bo1, bo2…"
        />
        <button
          type="button"
          className="ghost"
          disabled={busy}
          onClick={async () => {
            const path = await pickDirectory();
            if (path) await scan(path);
          }}
        >
          Chọn thư mục
        </button>
        <button
          type="button"
          className="primary"
          disabled={!sourceRoot.trim() || busy}
          onClick={() => scan(sourceRoot.trim())}
        >
          Quét
        </button>
      </div>
      {manifest && (
        <>
          <div className="row" style={{ marginTop: 10, justifyContent: "space-between" }}>
            <strong>{manifest.bundles.length} bundle · {manifest.ignoredPartnerFiles} partner bỏ qua</strong>
            <span className="hint">{manifest.ignoredHiddenFiles} file ẩn bỏ qua</span>
          </div>
          <div className="job-list" style={{ marginTop: 8, maxHeight: 330, overflow: "auto" }}>
            {manifest.bundles.map((bundle: PublishBundle) => {
              const checked = bundleIds.includes(bundle.id);
              return (
                <label key={bundle.id} className="job-card" style={{ cursor: "pointer" }}>
                  <div className="row" style={{ alignItems: "flex-start" }}>
                    <input
                      type="checkbox"
                      checked={checked}
                      onChange={(e) => {
                        setBundleIds((current) =>
                          e.target.checked
                            ? [...current, bundle.id]
                            : current.filter((id) => id !== bundle.id),
                        );
                      }}
                    />
                    <div style={{ flex: 1 }}>
                      <strong>{bundle.name}</strong>
                      <span className="pill">{bundle.images.length} ảnh</span>
                      <p className="hint" style={{ whiteSpace: "pre-wrap" }}>
                        {bundle.caption.slice(0, 180) || "(caption rỗng)"}
                      </p>
                    </div>
                  </div>
                </label>
              );
            })}
          </div>
          {manifest.notices.length > 0 && (
            <p className="hint" style={{ whiteSpace: "pre-wrap" }}>
              {manifest.notices.map((notice) => notice.message).join("\n")}
            </p>
          )}
        </>
      )}
      <section style={{ marginTop: 12 }}>
        <h3>Mapping tuần tự</h3>
        <div className="job-list">
          {selectedBundles.map((bundle, index) => (
            <article key={bundle.id} className="job-card">
              <strong>{index + 1}. {bundle.name}</strong>
              <span className="hint mono">→ {targets[index] ?? "Chưa có máy"}</span>
            </article>
          ))}
          {!selectedBundles.length && <p className="hint">Chọn bundle để tạo mapping.</p>}
        </div>
      </section>
      <label style={{ marginTop: 12 }}>
        Lịch chạy một lần (để trống = chạy ngay)
        <input type="datetime-local" value={runAt} onChange={(e) => setRunAt(e.target.value)} />
      </label>
      <p className="hint">Public · âm thanh mặc định · xoá asset sau khi có bằng chứng đăng thành công.</p>
      <button
        type="button"
        className="primary"
        disabled={!mappingReady || busy}
        onClick={async () => {
          setBusy(true);
          setMsg(null);
          try {
            const campaign = await publishCreateCampaign(
              sourceRoot.trim(),
              bundleIds,
              targets,
              runAt || null,
            );
            if (!runAt) await publishPrepare(campaign.id);
            await reload();
            setMsg(
              runAt
                ? `Đã lập lịch ${bundleIds.length} bundle cho ${targets.length} máy.`
                : `Đã chuẩn bị ${bundleIds.length} bundle. Bấm Post để đăng native trên TikTok.`,
            );
          } catch (e) {
            setMsg(String(e));
          } finally {
            setBusy(false);
          }
        }}
      >
        {runAt ? "Lập lịch" : "Tạo & chuẩn bị"} ({bundleIds.length} → {targets.length})
      </button>
      {msg && <p className="error" style={{ whiteSpace: "pre-wrap" }}>{msg}</p>}
      <div className="job-list" style={{ marginTop: 12 }}>
        {campaigns.map((campaign) => (
          <article key={campaign.id} className="job-card">
            <div>
              <strong>{campaign.id.slice(0, 8)}</strong>
              <span className={`pill ${campaign.state}`}>{campaign.state}</span>
            </div>
            <p className="hint">
              {campaign.assignments.length} mapping · {new Date(campaign.createdAt).toLocaleString()}
              {campaign.runAt ? ` · ${campaign.runAt}` : ""}
            </p>
            <div className="row">
              {campaign.state === "ready" && (
                <button
                  type="button"
                  className="primary"
                  disabled={busy}
                  onClick={async () => {
                    setBusy(true);
                    try {
                      await publishTransfer(campaign.id);
                      await reload();
                      setMsg("Đã import ảnh vào Photos. Bấm Post để mở composer TikTok.");
                    } catch (e) {
                      setMsg(String(e));
                    } finally {
                      setBusy(false);
                    }
                  }}
                >
                  Transfer media
                </button>
              )}
              {campaign.state === "imported" && (
                <button
                  type="button"
                  className="primary"
                  disabled={busy}
                  onClick={async () => {
                    setBusy(true);
                    try {
                      await publishPost(campaign.id);
                      await reload();
                      setMsg("Đã đăng và xác nhận frame TikTok; ảnh tạm đã được dọn.");
                    } catch (e) {
                      setMsg(String(e));
                    } finally {
                      setBusy(false);
                    }
                  }}
                >
                  Post
                </button>
              )}
              {(campaign.state === "queued" || campaign.state === "scheduled") && (
                <button
                  type="button"
                  className="ghost"
                  disabled={busy || campaign.state === "scheduled"}
                  onClick={async () => {
                    setBusy(true);
                    try {
                      await publishPrepare(campaign.id);
                      await reload();
                    } catch (e) {
                      setMsg(String(e));
                    } finally {
                      setBusy(false);
                    }
                  }}
                >
                  Prepare
                </button>
              )}
              {!['succeeded', 'cancelled', 'uncertain'].includes(campaign.state) && (
                <button
                  type="button"
                  className="ghost"
                  disabled={busy}
                  onClick={async () => {
                    setBusy(true);
                    try {
                      await publishCancel(campaign.id);
                      await reload();
                    } catch (e) {
                      setMsg(String(e));
                    } finally {
                      setBusy(false);
                    }
                  }}
                >
                  Huỷ
                </button>
              )}
            </div>
          </article>
        ))}
        {!campaigns.length && (
          <EmptyState
            compact
            icon={<IconRocket size={15} />}
            title="Chưa có chiến dịch"
            hint="Tạo chiến dịch ở khung bên trên để bắt đầu đăng bài."
          />
        )}
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
        <h2>Dữ liệu</h2>
        <button type="button" className="ghost" onClick={load}>
          Làm mới
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
          Làm mới
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
            {!users.length && (
              <EmptyState
                compact
                icon={<IconUser size={15} />}
                title="Chưa có người dùng"
                hint="Vẫn dùng được app ở chế độ khách."
              />
            )}
          </div>
        </section>
      </div>
    </div>
  );
}

export function LogsPage() {
  const [logs, setLogs] = useState<OpLog[]>([]);
  const load = () => listOpLogs(200).then(setLogs).catch((e) => flashError(e));
  useEffect(() => {
    load();
  }, []);
  return (
    <div className="panel">
      <header className="panel-header">
        <h2>Nhật ký thao tác</h2>
        <button type="button" className="ghost" onClick={load}>
          Làm mới
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
        {!logs.length && (
          <EmptyState
            compact
            icon={<IconLog size={15} />}
            title="Chưa có nhật ký"
            hint="Các thao tác Khởi động, Agent và Đăng bài sẽ được ghi lại ở đây."
          />
        )}
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
        <h2>Tài khoản</h2>
      </header>
      <section className="settings-card">
        <h3>Phiên đăng nhập</h3>
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
    reload().catch((e) => flashError(e));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <section style={{ marginTop: 16 }}>
      <h3>Lịch chạy</h3>
      <SelectionStrip
        devices={devices}
        selected={selected}
        onSelectAll={() => onSelectUdids(devices.map((d) => d.udid))}
        onClear={() => onSelectUdids([])}
        onSelectUdids={onSelectUdids}
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
            flashError(e);
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
