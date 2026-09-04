import { useEffect, useRef, useState } from "react";
import { AppWindow, FolderOpen, RefreshCw, Trash2 } from "lucide-react";

import { describeError } from "../describeError";
import {
  addAppLibrary,
  cancelAppInstallBatch,
  deleteAppLibrary,
  installLibraryAppBatch,
  listAppsLibrary,
  listGroups,
} from "../api";
import { SelectionStrip } from "../components/SelectionStrip";
import { flash, flashError } from "../farmToast";
import { targetsOf } from "../selectionTargets";
import { EmptyState, LoadingState, StatusNotice } from "../components/States";
import { IconApp } from "../components/Icons";
import {
  FormSection,
  ResponsiveTable,
  StatusChip,
  SummaryRail,
  type StatusTone,
} from "../components/WorkspacePrimitives";
import { pickFile } from "../pickFile";
import type { AppInstallResult, AppInstallStatus, AppLibraryItem, DeviceGroup } from "../types";
import type { SelProps } from "./pageProps";

const INSTALL_STATUS: Record<AppInstallStatus, { label: string; tone: StatusTone }> = {
  succeeded: { label: "Đã xác nhận", tone: "success" },
  beforeEffect: { label: "Chưa cài", tone: "warning" },
  failedVerified: { label: "Cài thất bại", tone: "error" },
  uncertain: { label: "Cần kiểm lại", tone: "warning" },
  cancelledBeforeDispatch: { label: "Đã hủy trước khi cài", tone: "neutral" },
};

function appVersion(app: AppLibraryItem): string {
  return app.versionName || app.version || "Chưa đọc được phiên bản";
}

/** The real app library and bounded, per-device installation results. */
export function AppsPage({ devices, selected, onSelectUdids }: SelProps) {
  const [items, setItems] = useState<AppLibraryItem[]>([]);
  const [path, setPath] = useState("");
  const [bundleId, setBundleId] = useState("");
  const [busy, setBusy] = useState(false);
  const [groups, setGroups] = useState<DeviceGroup[]>([]);
  const [groupId, setGroupId] = useState("");
  const [batchResults, setBatchResults] = useState<AppInstallResult[]>([]);
  const [activeBatch, setActiveBatch] = useState<{ id: string; appId: string } | null>(null);
  const [allowDowngrade, setAllowDowngrade] = useState(false);
  const [itemsLoading, setItemsLoading] = useState(true);
  const [itemsError, setItemsError] = useState<string | null>(null);
  const [groupsLoading, setGroupsLoading] = useState(true);
  const [groupsError, setGroupsError] = useState<string | null>(null);
  const libraryTicket = useRef(0);
  const groupsTicket = useRef(0);
  const iosDevices = devices.filter((device) => device.platform !== "android");
  const androidDevices = devices.filter((device) => device.platform === "android");

  const reloadLibrary = async () => {
    const ticket = ++libraryTicket.current;
    setItemsLoading(true);
    setItemsError(null);
    try {
      const next = await listAppsLibrary();
      if (ticket === libraryTicket.current) setItems(next);
    } catch (error) {
      if (ticket === libraryTicket.current) setItemsError(describeError(error));
    } finally {
      if (ticket === libraryTicket.current) setItemsLoading(false);
    }
  };

  const reloadGroups = async () => {
    const ticket = ++groupsTicket.current;
    setGroupsLoading(true);
    setGroupsError(null);
    try {
      const next = await listGroups();
      if (ticket === groupsTicket.current) setGroups(next);
    } catch (error) {
      if (ticket === groupsTicket.current) setGroupsError(describeError(error));
    } finally {
      if (ticket === groupsTicket.current) setGroupsLoading(false);
    }
  };

  useEffect(() => {
    void reloadLibrary();
    void reloadGroups();
    return () => {
      libraryTicket.current += 1;
      groupsTicket.current += 1;
    };
  }, []);

  const runBatch = async (app: AppLibraryItem, udids: string[]) => {
    if (!udids.length) return;
    if (allowDowngrade && !window.confirm(
      `Cho phép cài phiên bản ${appVersion(app)} thấp hơn bản đang có?`,
    )) return;
    const batchId = `app-install-${Date.now()}-${Math.random().toString(16).slice(2)}`;
    setBusy(true);
    setBatchResults([]);
    setActiveBatch({ id: batchId, appId: app.id });
    try {
      const response = await installLibraryAppBatch({
        batchId,
        appId: app.id,
        udids,
        allowDowngrade,
      });
      setBatchResults(response.results);
      const succeeded = response.results.filter((result) => result.status === "succeeded").length;
      const uncertain = response.results.filter((result) => result.status === "uncertain").length;
      const failed = response.results.length - succeeded - uncertain;
      flash(
        uncertain
          ? `Đã cài: ${succeeded} xác nhận, ${failed} thất bại, ${uncertain} cần kiểm lại`
          : `Đã cài: ${succeeded} xác nhận, ${failed} thất bại`,
      );
    } catch (error) {
      flashError(error);
    } finally {
      setActiveBatch(null);
      setBusy(false);
    }
  };

  const installToGroup = async (app: AppLibraryItem) => {
    const group = groups.find((candidate) => candidate.id === groupId);
    if (!group) {
      flash("Chọn một nhóm trước");
      return;
    }
    const platform = app.platform === "ios" ? iosDevices : androidDevices;
    const connected = new Set(platform.map((device) => device.udid));
    const targets = group.udids.filter((udid) => connected.has(udid));
    if (!targets.length) {
      flash(`Nhóm không có máy ${app.platform === "ios" ? "iPhone" : "Android"} đang kết nối`);
      return;
    }
    await runBatch(app, targets);
  };

  const selectedCount = selected.length || devices.length;
  const confirmedCount = batchResults.filter((result) => result.status === "succeeded").length;

  return (
    <div className="admin-workspace apps-workspace">
      <SelectionStrip
        devices={devices}
        selected={selected}
        onSelectAll={() => onSelectUdids(devices.map((device) => device.udid))}
        onClear={() => onSelectUdids([])}
        onSelectUdids={onSelectUdids}
      />

      <div className="admin-split">
        <main className="admin-main">
          <FormSection
            title="Thêm gói cài đặt"
            description="Đọc metadata và lưu một bản quản lý trước khi phân phối tới thiết bị."
          >
            <div className="admin-field-grid">
              <label className="is-wide">
                Tệp ứng dụng
                <input
                  value={path}
                  onChange={(event) => setPath(event.target.value)}
                  placeholder="Chọn .ipa, .apk, .xapk, .apkm hoặc .apks"
                />
              </label>
              <button
                type="button"
                className="ghost admin-field-action"
                onClick={async () => {
                  const selectedPath = await pickFile({
                    title: "Chọn ứng dụng",
                    filters: [{ name: "Ứng dụng", extensions: ["ipa", "apk", "xapk", "apkm", "apks"] }],
                  });
                  if (selectedPath) setPath(selectedPath);
                }}
              >
                <FolderOpen size={15} aria-hidden="true" />
                Chọn tệp
              </button>
              <label className="is-wide">
                Mã ứng dụng nếu metadata không có
                <input value={bundleId} onChange={(event) => setBundleId(event.target.value)} />
              </label>
              <div className="admin-actions admin-field-action">
                <button
                  type="button"
                  className="primary"
                  disabled={!path.trim() || busy}
                  onClick={async () => {
                    setBusy(true);
                    try {
                      await addAppLibrary(path.trim(), undefined, bundleId || undefined);
                      setPath("");
                      setBundleId("");
                      await reloadLibrary();
                      flash("Đã thêm ứng dụng vào thư viện");
                    } catch (error) {
                      flashError(error);
                    } finally {
                      setBusy(false);
                    }
                  }}
                >
                  <AppWindow size={15} aria-hidden="true" />
                  {busy ? "Đang xử lý…" : "Thêm vào thư viện"}
                </button>
              </div>
            </div>
          </FormSection>

          <FormSection
            title="Thư viện ứng dụng"
            description={items.length ? `${items.length} gói sẵn sàng phân phối` : undefined}
            actions={(
              <button type="button" className="icon-btn" onClick={() => void reloadLibrary()} disabled={itemsLoading} aria-label="Làm mới thư viện ứng dụng" title="Làm mới thư viện ứng dụng">
                <RefreshCw size={16} aria-hidden="true" />
              </button>
            )}
          >
            {itemsError && (
              <StatusNotice
                tone="error"
                action={<button type="button" className="ghost" onClick={() => void reloadLibrary()}>Thử lại thư viện ứng dụng</button>}
              >
                Không tải được thư viện ứng dụng: {itemsError}
              </StatusNotice>
            )}
            {itemsLoading && !items.length && <LoadingState label="Đang tải thư viện ứng dụng…" />}
            {!itemsLoading && !itemsError && !items.length && (
              <EmptyState
                compact
                icon={<IconApp size={15} />}
                title="Chưa có ứng dụng"
                hint="Chọn một gói cài đặt ở phần trên để bắt đầu."
              />
            )}
            {items.length > 0 && (
              <ResponsiveTable
                label="Thư viện ứng dụng"
                rows={items}
                keyForRow={(app) => app.id}
                columns={[
                  {
                    id: "app",
                    label: "Ứng dụng",
                    render: (app) => (
                      <span className="apps-library-name">
                        <strong>{app.name}</strong>
                        <small>{appVersion(app)}</small>
                      </span>
                    ),
                  },
                  {
                    id: "format",
                    label: "Nền tảng",
                    render: (app) => <StatusChip>{app.platform === "ios" ? "iPhone" : "Android"} · {app.packageFormat.toUpperCase()}</StatusChip>,
                  },
                  {
                    id: "metadata",
                    label: "Metadata",
                    render: (app) => (
                      <StatusChip tone={app.metadataError ? "warning" : "success"}>
                        {app.metadataError ? "Cần xem" : "Đã đọc"}
                      </StatusChip>
                    ),
                  },
                  {
                    id: "actions",
                    label: "Cài đặt",
                    render: (app) => {
                      const platformDevices = app.platform === "ios" ? iosDevices : androidDevices;
                      const installTargets = targetsOf(selected, platformDevices).filter((udid) =>
                        platformDevices.some((device) => device.udid === udid),
                      );
                      const platformName = app.platform === "ios" ? "iPhone" : "Android";
                      return (
                        <span className="admin-actions">
                          <button
                            type="button"
                            className="primary"
                            disabled={!installTargets.length || busy}
                            title={installTargets.length
                              ? `Cài lên ${installTargets.length} ${platformName}`
                              : app.platform === "ios"
                                ? "Không có iPhone nào để cài — IPA chỉ cài được lên iOS"
                                : "Không có Android nào để cài — gói Android chỉ cài được lên Android"}
                            onClick={() => void runBatch(app, installTargets)}
                          >
                            Cài → {installTargets.length} {platformName}
                          </button>
                          <button
                            type="button"
                            className="ghost"
                            disabled={!groupId || busy}
                            title="Cài lên toàn bộ máy trong nhóm đã chọn"
                            onClick={() => void installToGroup(app)}
                          >
                            Cài → nhóm
                          </button>
                          {activeBatch?.appId === app.id && (
                            <button type="button" className="ghost" onClick={() => void cancelAppInstallBatch(activeBatch.id)}>
                              Hủy máy chưa bắt đầu
                            </button>
                          )}
                          <details className="admin-detail">
                            <summary>Chi tiết</summary>
                            <dl>
                              <dt>Mã ứng dụng</dt><dd><code>{app.applicationId || app.bundleId || "Chưa có"}</code></dd>
                              <dt>Đường dẫn</dt><dd><code>{app.path}</code></dd>
                              {app.sha256 && <><dt>SHA-256</dt><dd><code>{app.sha256}</code></dd></>}
                              {app.metadataError && <><dt>Lỗi metadata</dt><dd>{app.metadataError}</dd></>}
                            </dl>
                          </details>
                          <button
                            type="button"
                            className="icon-btn"
                            aria-label={`Xóa ${app.name}`}
                            title={`Xóa ${app.name}`}
                            onClick={async () => {
                              if (!window.confirm(`Xóa ${app.name} khỏi thư viện?`)) return;
                              await deleteAppLibrary(app.id);
                              await reloadLibrary();
                            }}
                          >
                            <Trash2 size={15} aria-hidden="true" />
                          </button>
                        </span>
                      );
                    },
                  },
                ]}
              />
            )}
          </FormSection>

          {batchResults.length > 0 && (
            <FormSection title="Kết quả cài đặt" description={`${confirmedCount}/${batchResults.length} máy đã xác nhận phiên bản`}>
              <ResponsiveTable
                label="Kết quả cài đặt gần nhất"
                rows={batchResults}
                keyForRow={(result) => result.udid}
                columns={[
                  {
                    id: "device",
                    label: "Thiết bị",
                    render: (result) => devices.find((device) => device.udid === result.udid)?.name || "Thiết bị không còn trong danh sách",
                  },
                  {
                    id: "status",
                    label: "Kết quả",
                    render: (result) => <StatusChip tone={INSTALL_STATUS[result.status].tone}>{INSTALL_STATUS[result.status].label}</StatusChip>,
                  },
                  {
                    id: "version",
                    label: "Phiên bản đọc lại",
                    render: (result) => result.observedVersionName || "Chưa có",
                  },
                  {
                    id: "detail",
                    label: "Chi tiết",
                    render: (result) => result.detail ? <p className="admin-result-detail">{result.detail}</p> : "—",
                  },
                ]}
              />
            </FormSection>
          )}
        </main>

        <SummaryRail title="Phạm vi cài đặt">
          <dl className="admin-metric-grid">
            <div className="admin-metric"><dt>Đang nhắm tới</dt><dd>{selectedCount}</dd></div>
            <div className="admin-metric"><dt>Android kết nối</dt><dd>{androidDevices.length}</dd></div>
            <div className="admin-metric"><dt>iPhone kết nối</dt><dd>{iosDevices.length}</dd></div>
          </dl>
          {groupsError && (
            <StatusNotice tone="error" action={<button type="button" className="ghost" onClick={() => void reloadGroups()}>Thử lại danh sách nhóm</button>}>
              Không tải được danh sách nhóm: {groupsError}
            </StatusNotice>
          )}
          <label>
            Cài hàng loạt theo nhóm
            <select value={groupId} disabled={groupsLoading || !!groupsError} onChange={(event) => setGroupId(event.target.value)}>
              <option value="">
                {groupsLoading
                  ? "Đang tải danh sách nhóm…"
                  : groupsError
                    ? "Danh sách nhóm chưa tải được"
                    : groups.length
                      ? "Chọn nhóm"
                      : "Chưa có nhóm thiết bị"}
              </option>
              {groups.map((group) => <option key={group.id} value={group.id}>{group.name} ({group.udids.length} máy)</option>)}
            </select>
          </label>
          <label className="agent-toggle">
            <input type="checkbox" checked={allowDowngrade} disabled={busy} onChange={(event) => setAllowDowngrade(event.target.checked)} />
            Cho phép hạ phiên bản
          </label>
          <p className="hint">Hạ phiên bản luôn yêu cầu xác nhận riêng trước khi cài.</p>
        </SummaryRail>
      </div>
    </div>
  );
}
