import { lazy, Suspense, useRef, useState } from "react";
import {
  Copy,
  Download,
  Edit3,
  Plus,
  RefreshCw,
  Search,
  Sprout,
  MessagesSquare,
  Send,
  Upload,
  Trash2,
} from "lucide-react";
import {
  appWorkflowGet,
  appWorkflowList,
  appWorkflowTemplate,
  appWorkflowValidate,
  appWorkflowArchive,
  type AppWorkflowSummary,
  type AppWorkflowV1,
} from "../appWorkflow";
const AppWorkflowEditor = lazy(async()=>({default:(await import("../components/AppWorkflowEditor")).AppWorkflowEditor}));
import type { AutomationKind, DeviceInfo } from "../types";
import { describeError } from "../describeError";
import { requestConfirm } from "../confirmStore";
import { EmptyState, LoadingState, StatusNotice } from "../components/States";
import { useAsyncList } from "../useAsyncList";
const APPS = [
  { kind: "nurture", name: "Nuôi TikTok", Icon: Sprout },
  { kind: "interaction", name: "Tương tác", Icon: MessagesSquare },
  { kind: "publish", name: "Đăng bài", Icon: Send },
] as const;
export function MyAppsPage({
  devices,
  onOpenApp,
}: {
  devices: DeviceInfo[];
  onOpenApp: (kind: AutomationKind) => void;
}) {
  const [newKind, setNewKind] = useState<AutomationKind>("nurture");
  const [editor, setEditor] = useState<AppWorkflowV1 | null>(null),
    [search, setSearch] = useState(""),
    [error, setError] = useState<string | null>(null),
    [busy, setBusy] = useState(false);
  const importInput = useRef<HTMLInputElement>(null);
  const { data, error: loadError, loading, initialLoading, refreshing, load } = useAsyncList(appWorkflowList);
  const rows = data ?? [];
  const visible = rows.filter((row) => row.name.toLowerCase().includes(search.toLowerCase()));
  const visibleApps = APPS.filter((app) => app.name.toLowerCase().includes(search.toLowerCase()));
  const open = async (kind: AutomationKind, id?: string) => {
    setBusy(true);
    try {
      const doc = id
        ? await appWorkflowGet(id)
        : await appWorkflowTemplate(kind);
      if (!doc) throw new Error("Không tìm thấy phiên bản ứng dụng");
      setEditor(doc);
      setError(null);
    } catch (cause) {
      setError(describeError(cause));
    } finally {
      setBusy(false);
    }
  };
  const duplicate = async (row: AppWorkflowSummary) => {
    try {
      const doc = await appWorkflowGet(row.id);
      if (!doc) throw new Error("Không tìm thấy ứng dụng");
      setEditor({
        ...doc,
        id: crypto.randomUUID(),
        revision: 0,
        name: doc.name + " — bản sao",
      });
    } catch (cause) {
      setError(describeError(cause));
    }
  };
  const exportApp = async (row: AppWorkflowSummary) => {
    try {
      const doc = await appWorkflowGet(row.id);
      const url = URL.createObjectURL(
        new Blob([JSON.stringify(doc, null, 2)], { type: "application/json" }),
      );
      const a = document.createElement("a");
      a.href = url;
      a.download = `riviu-app-${row.kind}.json`;
      a.click();
      URL.revokeObjectURL(url);
    } catch (cause) {
      setError(describeError(cause));
    }
  };
  if (editor)
    return (
      <Suspense fallback={<LoadingState label="Đang mở trình thiết kế…" />}><AppWorkflowEditor
        key={editor.id}
        initial={editor}
        devices={devices}
        onBack={() => {
          setEditor(null);
          void load();
        }}
        onSaved={() => void load()}
      /></Suspense>
    );
  return (
    <section className="my-app-library" aria-label="My Apps">
      <header className="operator-toolbar">
        <label>
          <Search size={17} aria-hidden="true" />
          <input
            type="search"
            aria-label="Tìm ứng dụng"
            placeholder="Tìm ứng dụng…"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
          />
        </label>
        <div className="grow" />
        <button type="button" aria-label="Làm mới ứng dụng" title="Làm mới ứng dụng" disabled={loading} onClick={() => void load()}>
          <RefreshCw size={16} aria-hidden="true" />
        </button>
        <select aria-label="Loại ứng dụng mới" value={newKind} onChange={event=>setNewKind(event.target.value as AutomationKind)}>{APPS.map(app=><option key={app.kind} value={app.kind}>{app.name}</option>)}</select>
        <button
          type="button"
          onClick={() => void open(newKind)}
          className="primary"
        >
          <Plus size={16} />
          Ứng dụng mới
        </button>
      </header>
      {error && <StatusNotice tone="error">{error}</StatusNotice>}
      <div className="operator-toolbar">
        <button type="button" aria-label="Nhập ứng dụng từ JSON" onClick={() => importInput.current?.click()}>
          <Upload size={15} aria-hidden="true" />
          Nhập ứng dụng
        </button>
        <input
          ref={importInput}
          type="file"
          accept=".json"
          hidden
          onChange={async (event) => {
            const input = event.currentTarget;
            const file = input.files?.[0];
            if (!file) return;
            try {
              if (file.size > 2_000_000)
                throw new Error("Tệp ứng dụng vượt quá 2 MB");
              const doc = JSON.parse(await file.text()) as AppWorkflowV1;
              await appWorkflowValidate(doc);
              setEditor({ ...doc, id: crypto.randomUUID(), revision: 0 });
              setError(null);
            } catch (cause) {
              setError(describeError(cause));
            } finally {
              input.value = "";
            }
          }}
        />
        <span>3 ứng dụng có sẵn{data !== undefined ? ` · ${rows.length} quy trình đã lưu` : " · Chưa tải quy trình đã lưu"}</span>
      </div>
      <table className="builtin-app-list">
        <thead>
          <tr>
            <th>Tên ứng dụng</th>
            <th>Nguồn</th>
            <th>Phiên bản</th>
            <th>Thao tác</th>
          </tr>
        </thead>
        <tbody>
          {visibleApps.map(({ kind, name, Icon }) => {
            const current = rows.find((row) => row.kind === kind);
            return (
              <tr key={kind}>
                <td>
                  <Icon size={21} />
                  <button
                    type="button"
                    className="ghost"
                    onClick={() => onOpenApp(kind)}
                  >
                    {name}
                  </button>
                </td>
                <td>Riviu</td>
                <td>{current?.latestRevision ?? "Mẫu"}</td>
                <td>
                  <button
                    type="button"
                    disabled={busy}
                    onClick={() => void open(kind, current?.id)}
                  >
                    <Edit3 size={15} />
                    Thêm Flow
                  </button>
                  <button type="button" onClick={() => onOpenApp(kind)}>
                    Mở chức năng
                  </button>
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
      {initialLoading && <LoadingState label="Đang tải quy trình đã lưu…" />}
      {refreshing && <LoadingState label="Đang làm mới quy trình đã lưu…" />}
      {loadError && (
        <StatusNotice tone="error" action={<button type="button" onClick={() => void load()}>Thử lại</button>}>
          Không tải được quy trình đã lưu: {loadError}{data !== undefined && " · Đang giữ dữ liệu lần tải trước."}
        </StatusNotice>
      )}
      {!loading && !loadError && data !== undefined && rows.length === 0 && (
        <EmptyState compact title="Chưa có quy trình đã lưu" hint="Tạo ứng dụng mới hoặc nhập JSON để thêm quy trình. Ba ứng dụng có sẵn vẫn sử dụng được." />
      )}
      {data !== undefined && search !== "" && visible.length === 0 && (rows.length > 0 || visibleApps.length === 0) && (
        <EmptyState compact title={visibleApps.length > 0 ? "Không có quy trình đã lưu khớp tìm kiếm" : "Không có ứng dụng khớp tìm kiếm"} action={<button type="button" onClick={() => setSearch("")}>Xóa tìm kiếm</button>} />
      )}
      {visible.length > 0 && (
        <table aria-label="Quy trình đã lưu" aria-busy={refreshing}>
          <thead>
            <tr>
              <th>Quy trình đã lưu</th>
              <th>Loại</th>
              <th>Phiên bản</th>
              <th>Cập nhật</th>
              <th>Thao tác</th>
            </tr>
          </thead>
          <tbody>
            {visible.map((row) => (
                <tr key={row.id}>
                  <td>
                    <button
                      type="button"
                      className="ghost"
                      onClick={() => void open(row.kind, row.id)}
                    >
                      {row.name}
                    </button>
                  </td>
                  <td>{APPS.find((a) => a.kind === row.kind)?.name}</td>
                  <td>{row.latestRevision}</td>
                  <td>{new Date(row.updatedAt).toLocaleString("vi-VN")}</td>
                  <td>
                    <button
                      type="button"
                      title="Nhân bản"
                      onClick={() => void duplicate(row)}
                    >
                      <Copy size={16} />
                    </button>
                    <button
                      type="button"
                      title="Xuất ứng dụng"
                      onClick={() => void exportApp(row)}
                    >
                      <Download size={16} />
                    </button>
                    <button
                      type="button"
                      title="Lưu trữ ứng dụng"
                      onClick={async () => {
                        if (
                          await requestConfirm({
                            title: "Lưu trữ ứng dụng?",
                            message: row.name,
                            confirmLabel: "Lưu trữ",
                          })
                        ) {
                          try {
                            await appWorkflowArchive(
                              row.id,
                              row.latestRevision,
                            );
                            setError(null);
                            await load();
                          } catch (cause) {
                            setError(describeError(cause));
                          }
                        }
                      }}
                    >
                      <Trash2 size={16} />
                    </button>
                  </td>
                </tr>
              ))}
          </tbody>
        </table>
      )}
    </section>
  );
}
