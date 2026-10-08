import { machineNumberLabel } from "../deviceNaming";
import { useEffect, useRef, useState } from "react";
import { AppWindow, FolderOpen, Plus, RefreshCw, Trash2 } from "lucide-react";

import { describeError } from "../describeError";
import { requestConfirm } from "../confirmStore";
import {
  addAppLibrary,
  cancelAppInstallBatch,
  deleteAppLibrary,
  installLibraryAppBatch,
  listAppsLibrary,
  listGroups,
  uninstallLibraryAppBatch,
} from "../api";
import { TargetSelector } from "../components/TargetSelector";
import { LibraryBatchMonitor } from "../components/LibraryBatchMonitor";
import { useLibraryBatch } from "../useLibraryBatch";
import { flash, flashError } from "../farmToast";
import { resolveAutomationTarget } from "../automationTargets";
import { EmptyState, LoadingState, StatusNotice } from "../components/States";
import { IconApp } from "../components/Icons";
import {
  FormSection,
  DetailDrawer,
  ResponsiveTable,
  StatusChip,
  SummaryRail,
  type StatusTone,
} from "../components/WorkspacePrimitives";
import { pickFile } from "../pickFile";
import type {
  AppInstallResult,
  AppInstallStatus,
  AppLibraryItem,
  AppRemovalMode,
  AppRemovalOutcome,
  AppRemovalResult,
  DeviceGroup,
  TargetRef,
} from "../types";
import type { SelProps } from "./pageProps";
import type { OperationSourceRef } from "../operationSource";

const INSTALL_STATUS: Record<AppInstallStatus, { label: string; tone: StatusTone }> = {
  succeeded: { label: "Đã xác nhận", tone: "success" },
  beforeEffect: { label: "Chưa cài", tone: "warning" },
  failedVerified: { label: "Cài thất bại", tone: "error" },
  uncertain: { label: "Cần kiểm lại", tone: "warning" },
  cancelledBeforeDispatch: { label: "Đã hủy trước khi cài", tone: "neutral" },
};

const REMOVAL_OUTCOME: Record<AppRemovalOutcome, { label: string; tone: StatusTone }> = {
  done: { label: "Đã xong", tone: "success" },
  refusedBusy: { label: "Máy bận, chưa gỡ", tone: "warning" },
  failedBeforeEffect: { label: "Chưa gỡ", tone: "error" },
  unknownAfterDispatch: { label: "Cần kiểm lại", tone: "warning" },
};

const REMOVAL_VERB: Record<AppRemovalMode, string> = { uninstall: "Gỡ", reinstall: "Gỡ và cài lại" };

function appVersion(app: AppLibraryItem): string {
  return app.versionName || app.version || "Chưa đọc được phiên bản";
}

/** The real app library and bounded, per-device installation results. */
export function AppsPage({ devices, selected, operationSource, deviceNumbers }: SelProps & { operationSource?: OperationSourceRef; deviceNumbers?: ReadonlyMap<string, number> }) {
  const batch = useLibraryBatch("appInstall", operationSource?.kind === "appInstall" ? operationSource.operationId : undefined);
  const [importOpen, setImportOpen] = useState(false);
  const [items, setItems] = useState<AppLibraryItem[]>([]);
  const [path, setPath] = useState("");
  const [bundleId, setBundleId] = useState("");
  const [busy, setBusy] = useState(false);
  const [groups, setGroups] = useState<DeviceGroup[]>([]);
  const [targetRef, setTargetRef] = useState<TargetRef>(() => ({ type: "explicit", udids: [...selected] }));
  const [batchResults, setBatchResults] = useState<AppInstallResult[]>([]);
  const [batchLabels, setBatchLabels] = useState<Map<string,string>>(new Map());
  const [activeBatch, setActiveBatch] = useState<{ id: string; appId: string } | null>(null);
  const [removal, setRemoval] = useState<{ mode: AppRemovalMode; appName: string; results: AppRemovalResult[] } | null>(null);
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
    if (allowDowngrade && !(await requestConfirm({
      title: "Cho phép hạ phiên bản?",
      message: `Cài ${appVersion(app)} có thể thay thế phiên bản mới hơn trên ${udids.length} thiết bị. Dữ liệu ứng dụng được giữ nguyên.`,
      confirmLabel: "Tiếp tục cài",
      danger: true,
    }))) return;
    const batchId = `app-install-${Date.now()}-${Math.random().toString(16).slice(2)}`;
    setBusy(true);
    setBatchResults([]);
    setBatchLabels(new Map(udids.map(udid => [udid, `${machineNumberLabel(deviceNumbers?.get(udid))} · ${devices.find(device => device.udid === udid)?.name ?? udid}`])));
    setActiveBatch({ id: batchId, appId: app.id });
    try {
      const response = await installLibraryAppBatch({
        batchId,
        appId: app.id,
        udids,
        allowDowngrade,
      });
      setBatchResults(response.results);
      if (operationSource) batch.follow(`appInstall:${response.batchId}`);
      if (response.target) setBatchLabels(new Map(response.target.included.map((device) => [device.udid,
        `${machineNumberLabel(device.number)}${device.alias.trim() ? ` · ${device.alias.trim()}` : ""}`])));
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
      void batch.reload();
    }
  };

  /**
   * Uninstall is destructive: the app's data and the account logged in on that phone go with it.
   * One confirm names that loss; the backend then gives every phone one attempt and never retries
   * a dispatched uninstall or install on its own.
   */
  const runRemoval = async (app: AppLibraryItem, udids: string[], mode: AppRemovalMode) => {
    if (!udids.length) return;
    const confirmed = await requestConfirm({
      title: mode === "uninstall" ? `Gỡ ${app.name} khỏi ${udids.length} máy?` : `Gỡ và cài lại ${app.name} trên ${udids.length} máy?`,
      message: `Dữ liệu của ${app.name} và tài khoản đang đăng nhập trong ứng dụng trên ${udids.length} máy sẽ bị mất`
        + (mode === "reinstall" ? `; sau đó cài lại ${appVersion(app)} từ thư viện.` : ".")
        + " Máy đang đăng bài, nuôi hoặc tương tác sẽ bị từ chối, không chờ. Lệnh đã gửi sẽ không tự gửi lại.",
      confirmLabel: mode === "uninstall" ? "Gỡ cài đặt" : "Gỡ và cài lại",
      danger: true,
    });
    if (!confirmed) return;
    setBusy(true);
    setRemoval(null);
    setBatchLabels(new Map(udids.map(udid => [udid, `${machineNumberLabel(deviceNumbers?.get(udid))} · ${devices.find(device => device.udid === udid)?.name ?? udid}`])));
    try {
      const response = await uninstallLibraryAppBatch({ appId: app.id, udids, mode });
      setRemoval({ mode, appName: app.name, results: response.results });
      const count = (outcome: AppRemovalOutcome) => response.results.filter((result) => result.outcome === outcome).length;
      flash(`${REMOVAL_VERB[mode]}: ${count("done")} xong, ${count("refusedBusy")} máy bận, ${count("failedBeforeEffect")} chưa gỡ, ${count("unknownAfterDispatch")} cần kiểm lại`);
    } catch (error) {
      flashError(error);
    } finally {
      setBusy(false);
    }
  };

  const targets = resolveAutomationTarget(targetRef, devices, groups);
  const selectedCount = targets.length;
  const confirmedCount = batchResults.filter((result) => result.status === "succeeded").length;

  return (
    <div className="admin-workspace apps-workspace">
      <TargetSelector deviceLabel={device => `${machineNumberLabel(deviceNumbers?.get(device.udid))} · ${device.name}`} devices={devices} groups={groups} selected={[]} onChange={() => undefined}
        targetRef={targetRef} onTargetRefChange={setTargetRef} requireChoice label="Phạm vi cài đặt" />
      {groupsLoading && <LoadingState label="Đang tải danh sách nhóm…" />}
      {groupsError && <StatusNotice tone="error" action={<button type="button" className="ghost" onClick={() => void reloadGroups()}>Thử lại danh sách nhóm</button>}>
        Không tải được danh sách nhóm: {groupsError}
      </StatusNotice>}

      <div className="admin-split">
        <main className="admin-main">
          <DetailDrawer open={importOpen} title="Thêm gói cài đặt" onClose={() => { if (!busy) setImportOpen(false); }}>
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
                      setImportOpen(false);
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
          </DetailDrawer>

          <FormSection
            title="Thư viện ứng dụng"
            description={items.length ? `${items.length} gói sẵn sàng phân phối` : undefined}
            actions={(
              <><button type="button" className="primary" onClick={() => setImportOpen(true)}><Plus size={15} aria-hidden="true" />Thêm gói</button>
              <button type="button" className="icon-btn" onClick={() => void reloadLibrary()} disabled={itemsLoading} aria-label="Làm mới thư viện ứng dụng" title="Làm mới thư viện ứng dụng">
                <RefreshCw size={16} aria-hidden="true" />
              </button></>
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
                action={<button type="button" className="primary" onClick={() => setImportOpen(true)}>Thêm gói cài đặt</button>}
              />
            )}
            {items.length > 0 && (
              <ResponsiveTable
                label="Thư viện ứng dụng"
                viewKey="apps"
                searchText={(app) => [app.name, appVersion(app), app.applicationId, app.bundleId, app.platform].join(" ")}
                rows={items}
                keyForRow={(app) => app.id}
                columns={[
                  {
                    id: "app",
                    label: "Ứng dụng",
                    sortValue: (app) => app.name,
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
                    sortValue: (app) => app.platform,
                    render: (app) => <StatusChip>{app.platform === "ios" ? "iPhone" : "Android"} · {app.packageFormat.toUpperCase()}</StatusChip>,
                  },
                  {
                    id: "metadata",
                    label: "Thông tin gói",
                    render: (app) => (
                      <StatusChip tone={app.metadataError ? "warning" : "success"}>
                        {app.metadataError ? "Cần xem" : "Đã đọc"}
                      </StatusChip>
                    ),
                  },
                  {
                    id: "actions",
                    label: "Cài đặt",
                    required: true,
                    render: (app) => {
                      const platformDevices = app.platform === "ios" ? iosDevices : androidDevices;
                      const installTargets = targets.filter((udid) =>
                        platformDevices.some((device) => device.udid === udid),
                      );
                      const platformName = app.platform === "ios" ? "iPhone" : "Android";
                      return (
                        <span className="admin-actions">
                          <button
                            type="button"
                            className="ghost"
                            disabled={!installTargets.length || busy || batch.loading || batch.active || !!batch.error}
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
                            disabled={!installTargets.length || busy || batch.active}
                            title={`Gỡ ${app.name} khỏi ${installTargets.length} ${platformName}; dữ liệu và tài khoản trong ứng dụng sẽ mất`}
                            onClick={() => void runRemoval(app, installTargets, "uninstall")}
                          >
                            Gỡ → {installTargets.length} {platformName}
                          </button>
                          <button
                            type="button"
                            className="ghost"
                            disabled={!installTargets.length || busy || batch.active}
                            title={`Gỡ rồi cài lại đúng gói ${appVersion(app)} trong thư viện lên ${installTargets.length} ${platformName}`}
                            onClick={() => void runRemoval(app, installTargets, "reinstall")}
                          >
                            Cài lại → {installTargets.length} {platformName}
                          </button>
                          {activeBatch?.appId === app.id && (
                            <button type="button" className="ghost" onClick={() => void cancelAppInstallBatch(activeBatch.id).catch(flashError)}>
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
                            disabled={busy || batch.active}
                            onClick={async () => {
                              const confirmed = await requestConfirm({
                                title: `Xóa ${app.name}?`,
                                message: "Gói cài đặt sẽ bị xóa khỏi thư viện. Ứng dụng trên thiết bị không bị ảnh hưởng.",
                                confirmLabel: "Xóa khỏi thư viện",
                                danger: true,
                              });
                              if (!confirmed) return;
                              try {
                                await deleteAppLibrary(app.id);
                                await reloadLibrary();
                              } catch (cause) { flashError(cause); }
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

          <LibraryBatchMonitor batch={batch} retryDisabled={busy} onRetry={(artifactId,udids) => {
            const app = items.find((item) => item.id === artifactId);
            if (app) void runBatch(app,udids);
            else flash("Gói cài đặt không còn trong thư viện; hãy thêm lại trước khi chạy.");
          }} />
          {!batch.detail && batchResults.length > 0 && (
            <FormSection title="Kết quả cài đặt" description={`${confirmedCount}/${batchResults.length} máy đã xác nhận phiên bản`}>
              <ResponsiveTable
                label="Kết quả cài đặt gần nhất"
                rows={batchResults}
                keyForRow={(result) => result.udid}
                columns={[
                  {
                    id: "device",
                    label: "Thiết bị",
                    render: (result) => batchLabels.get(result.udid) ?? "Máy trong lần chạy",
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
          {removal && removal.results.length > 0 && (
            <FormSection
              title={removal.mode === "uninstall" ? "Kết quả gỡ cài đặt" : "Kết quả gỡ và cài lại"}
              description={`${removal.appName} · ${removal.results.filter((result) => result.outcome === "done").length}/${removal.results.length} máy đã xong`}
            >
              <ResponsiveTable
                label="Kết quả gỡ gần nhất"
                rows={removal.results}
                keyForRow={(result) => result.udid}
                columns={[
                  {
                    id: "device",
                    label: "Thiết bị",
                    render: (result) => batchLabels.get(result.udid) ?? "Máy trong lần chạy",
                  },
                  {
                    id: "status",
                    label: "Kết quả",
                    render: (result) => <StatusChip tone={REMOVAL_OUTCOME[result.outcome].tone}>{REMOVAL_OUTCOME[result.outcome].label}</StatusChip>,
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
          <p className="hint">Chọn gói trong thư viện để cài lên các máy cùng nền tảng trong phạm vi này.</p>
          <label className="agent-toggle">
            <input type="checkbox" checked={allowDowngrade} disabled={busy} onChange={(event) => setAllowDowngrade(event.target.checked)} />
            Cho phép hạ phiên bản
          </label>
          <p className="hint">Hạ phiên bản luôn yêu cầu xác nhận riêng trước khi cài.</p>
          <p className="hint">Gỡ và cài lại làm mất dữ liệu và tài khoản trong ứng dụng; máy đang bận bị từ chối, không chờ.</p>
        </SummaryRail>
      </div>
    </div>
  );
}
