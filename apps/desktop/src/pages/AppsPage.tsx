import { useEffect, useRef, useState } from "react";
import { describeError } from "../describeError";
import {
  addAppLibrary,
  deleteAppLibrary,
  installIpaToGroup,
  installLibraryApp,
  listAppsLibrary,
  listGroups,
} from "../api";
import { SelectionStrip } from "../components/SelectionStrip";
import { flash, flashError } from "../farmToast";
import { targetsOf } from "../selectionTargets";
import { EmptyState, LoadingState, StatusNotice } from "../components/States";
import { IconApp } from "../components/Icons";
import { pickIpa } from "../pickFile";
import type { AppLibraryItem, DeviceGroup, GroupInstallResult } from "../types";
import type { SelProps } from "./pageProps";

/** The app library and installing from it across a selection. */
export function AppsPage({ devices, selected, onSelectUdids }: SelProps) {
  const [items, setItems] = useState<AppLibraryItem[]>([]);
  const [path, setPath] = useState("");
  const [bundleId, setBundleId] = useState("");
  const [busy, setBusy] = useState(false);
  const [groups, setGroups] = useState<DeviceGroup[]>([]);
  const [groupId, setGroupId] = useState("");
  const [groupResults, setGroupResults] = useState<GroupInstallResult[]>([]);
  const [itemsLoading, setItemsLoading] = useState(true);
  const [itemsError, setItemsError] = useState<string | null>(null);
  const [groupsLoading, setGroupsLoading] = useState(true);
  const [groupsError, setGroupsError] = useState<string | null>(null);
  const libraryTicket = useRef(0);
  const groupsTicket = useRef(0);
  // **iPhones only, and never the whole fleet by default.** This library holds `.ipa`
  // files; `targetsOf` falls back to every connected device when nothing is selected, so
  // an unselected click used to push an iOS app at every Android serial in the room and
  // collect one failure per phone. Android apps are installed from the device overlay's
  // "Cài APK", which is a different file and a different command.
  const iosDevices = devices.filter((device) => device.platform !== "android");
  const targets = targetsOf(selected, iosDevices).filter((udid) =>
    iosDevices.some((device) => device.udid === udid),
  );
  const androidSelected = selected.filter((udid) =>
    devices.some((device) => device.udid === udid && device.platform === "android"),
  ).length;

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
      <SelectionStrip
        devices={devices}
        selected={selected}
        onSelectAll={() => onSelectUdids(devices.map((d) => d.udid))}
        onClear={() => onSelectUdids([])}
        onSelectUdids={onSelectUdids}
      />
      {androidSelected > 0 && (
        <p className="hint" role="status">
          Bỏ qua {androidSelected} máy Android đang chọn — thư viện này là IPA, chỉ cài được
          lên iPhone. Cài APK cho Android trong menu điều khiển của từng máy.
        </p>
      )}
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
        Bundle ID (không bắt buộc)
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
        {itemsError && (
          <StatusNotice
            tone="error"
            action={(
              <button type="button" className="ghost" onClick={() => void reloadLibrary()}>
                Thử lại thư viện IPA
              </button>
            )}
          >
            Không tải được thư viện IPA: {itemsError}
          </StatusNotice>
        )}
        {itemsLoading && !items.length && <LoadingState label="Đang tải thư viện IPA…" />}
        {items.map((a) => (
          <article key={a.id} className="job-card">
            <div>
              <strong>{a.name}</strong>
              <span className="pill">{a.bundleId || "chưa có bundle ID"}</span>
            </div>
            <p className="hint">{a.path}</p>
            <div className="row">
              <button
                type="button"
                className="primary"
                disabled={!targets.length || busy}
                title={
                  targets.length
                    ? `Cài lên ${targets.length} iPhone`
                    : "Không có iPhone nào để cài — IPA chỉ cài được lên iOS"
                }
                onClick={async () => {
                  setBusy(true);
                  try {
                    const errors: string[] = [];
                    for (const u of targets) {
                      try {
                        await installLibraryApp(u, a.id);
                      } catch (e) {
                        // `${e}` on a Tauri rejection ({ code, message }) renders as
                        // "[object Object]", which loses the only actionable half of the
                        // message. `describeError` exists for exactly this.
                        errors.push(`${u.slice(0, 8)}: ${describeError(e)}`);
                      }
                    }
                    if (errors.length) flash(`Một số máy lỗi:\n${errors.join("\n")}`);
                    else flash(`Đã cài lên ${targets.length} iPhone`);
                  } finally {
                    setBusy(false);
                  }
                }}
              >
                Install → {targets.length} iPhone
              </button>
              <button
                type="button"
                className="primary"
                disabled={!groupId || busy}
                title="Cài lên toàn bộ máy trong nhóm đã chọn"
                onClick={() => installToGroup(a.path)}
              >
                Cài → nhóm
              </button>
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
          </article>
        ))}
        {!itemsLoading && !itemsError && !items.length && (
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
