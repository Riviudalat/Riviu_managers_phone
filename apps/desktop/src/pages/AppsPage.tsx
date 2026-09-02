import { useEffect, useRef, useState } from "react";
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
import { pickFile } from "../pickFile";
import type { AppInstallResult, AppInstallStatus, AppLibraryItem, DeviceGroup } from "../types";
import type { SelProps } from "./pageProps";

/** The app library and installing from it across a selection. */
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
      `Cho phép cài phiên bản ${app.versionName || app.version || "đã chọn"} thấp hơn bản đang có?`,
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
    } catch (e) {
      flashError(e);
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

  const installStatus: Record<AppInstallStatus, string> = {
    succeeded: "Đã xác nhận",
    beforeEffect: "Chưa cài",
    failedVerified: "Cài thất bại",
    uncertain: "Cần kiểm lại",
    cancelledBeforeDispatch: "Đã hủy trước khi cài",
  };

  return (
    <div className="panel">
      <SelectionStrip
        devices={devices}
        selected={selected}
        onSelectAll={() => onSelectUdids(devices.map((d) => d.udid))}
        onClear={() => onSelectUdids([])}
        onSelectUdids={onSelectUdids}
      />
      {groupsError && (
        <StatusNotice
          tone="error"
          action={(
            <button type="button" className="ghost" onClick={() => void reloadGroups()}>
              Thử lại danh sách nhóm
            </button>
          )}
        >
          Không tải được danh sách nhóm: {groupsError}
        </StatusNotice>
      )}
      <div className="row" style={{ marginTop: 8 }}>
        <label style={{ flex: 1 }}>
          Cài hàng loạt theo nhóm
          <select
            value={groupId}
            disabled={groupsLoading || !!groupsError}
            onChange={(e) => setGroupId(e.target.value)}
          >
            <option value="">
              {groupsLoading
                ? "Đang tải danh sách nhóm…"
                : groupsError
                  ? "Danh sách nhóm chưa tải được"
                  : groups.length
                  ? "— chọn nhóm —"
                  : "Chưa có nhóm thiết bị"}
            </option>
            {groups.map((g) => (
              <option key={g.id} value={g.id}>
                {g.name} ({g.udids.length} máy)
              </option>
            ))}
          </select>
        </label>
      </div>
      <label className="row" style={{ marginTop: 8 }}>
        <input
          type="checkbox"
          checked={allowDowngrade}
          disabled={busy}
          onChange={(event) => setAllowDowngrade(event.target.checked)}
        />
        Cho phép hạ phiên bản
      </label>
      <div className="row" style={{ marginTop: 8 }}>
        <input
          style={{ flex: 1 }}
          value={path}
          onChange={(e) => setPath(e.target.value)}
          placeholder="Đường dẫn .ipa, .apk, .xapk, .apkm hoặc .apks…"
        />
        <button
          type="button"
          className="ghost"
          onClick={async () => {
            const p = await pickFile({
              title: "Chọn ứng dụng",
              filters: [{ name: "Ứng dụng", extensions: ["ipa", "apk", "xapk", "apkm", "apks"] }],
            });
            if (p) setPath(p);
          }}
        >
          Chọn ứng dụng…
        </button>
      </div>
      <label>
        Mã ứng dụng (không bắt buộc)
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
            await reloadLibrary();
            flash("Đã thêm ứng dụng vào thư viện");
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
        {itemsError && (
          <StatusNotice
            tone="error"
            action={(
              <button type="button" className="ghost" onClick={() => void reloadLibrary()}>
                Thử lại thư viện ứng dụng
              </button>
            )}
          >
          Không tải được thư viện ứng dụng: {itemsError}
          </StatusNotice>
        )}
        {itemsLoading && !items.length && <LoadingState label="Đang tải thư viện ứng dụng…" />}
        {items.map((a) => {
          const platformDevices = a.platform === "ios" ? iosDevices : androidDevices;
          const installTargets = targetsOf(selected, platformDevices).filter((udid) =>
            platformDevices.some((device) => device.udid === udid),
          );
          const platformName = a.platform === "ios" ? "iPhone" : "Android";
          return <article key={a.id} className="job-card">
            <div>
              <strong>{a.name}</strong>
              <span className="pill">{a.bundleId || "chưa có bundle ID"}</span>
              <span className="pill">{a.packageFormat.toUpperCase()}</span>
            </div>
            <p className="hint">{a.path}</p>
            <div className="row">
              <button
                type="button"
                className="primary"
                disabled={!installTargets.length || busy}
                title={
                  installTargets.length
                    ? `Cài lên ${installTargets.length} ${platformName}`
                    : a.platform === "ios"
                      ? "Không có iPhone nào để cài — IPA chỉ cài được lên iOS"
                      : "Không có Android nào để cài — gói Android chỉ cài được lên Android"
                }
                onClick={() => void runBatch(a, installTargets)}
              >
                Cài → {installTargets.length} {platformName}
              </button>
              <button
                type="button"
                className="primary"
                disabled={!groupId || busy}
                title="Cài lên toàn bộ máy trong nhóm đã chọn"
                onClick={() => installToGroup(a)}
              >
                Cài → nhóm
              </button>
              {activeBatch?.appId === a.id && (
                <button
                  type="button"
                  className="ghost"
                  onClick={() => void cancelAppInstallBatch(activeBatch.id)}
                >
                  Hủy máy chưa bắt đầu
                </button>
              )}
              <button
                type="button"
                className="ghost"
                onClick={async () => {
                  await deleteAppLibrary(a.id);
                  await reloadLibrary();
                }}
              >
                Xóa
              </button>
            </div>
          </article>;
        })}
        {!itemsLoading && !itemsError && !items.length && (
          <EmptyState
            compact
            icon={<IconApp size={15} />}
            title="Chưa có ứng dụng"
            hint="Bấm «Chọn ứng dụng…» để thêm gói iOS hoặc Android vào thư viện."
          />
        )}
      </div>
      {batchResults.length > 0 && (
        <div className="job-list" style={{ marginTop: 12 }}>
          <h4>Kết quả cài đặt</h4>
          {batchResults.map((result) => {
            const device = devices.find((candidate) => candidate.udid === result.udid);
            return <article key={result.udid} className="job-card">
              <div>
                <span className="pill">{installStatus[result.status]}</span>
                <strong>{device?.name || result.udid.slice(0, 12)}</strong>
              </div>
              {result.detail && <p className="hint">{result.detail}</p>}
            </article>;
          })}
        </div>
      )}
    </div>
  );
}
