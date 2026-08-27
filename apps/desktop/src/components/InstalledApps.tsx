import { useEffect, useState } from "react";
import { launchDeviceApp, listInstalledApps } from "../api";
import { describeError } from "../describeError";
import { pushToast, toastError } from "../toastStore";
import { IconRefresh } from "./Icons";
import {
  installedAppsFootnote,
  installedAppsView,
  type InstalledAppsLoad,
} from "../installedAppsView";

interface Props {
  udid: string;
  deviceName: string;
  /**
   * Draw the compact always-visible column the reference product has (icon, name, click to
   * launch) instead of the plain listing. The overlay panel passes this; nothing else does.
   */
  launchable?: boolean;
}

/**
 * What is installed on one phone — the App List.
 *
 * Shaped after the reference product's own panel, and that shape is the point: the list is
 * **always there** under the function rows, one row per app with its real icon and name, and a
 * click launches it. It used to hide behind a "Ứng dụng" toggle and be read-only, so an
 * operator looking for the phone's apps found a menu row instead of a list — and finding an
 * app still left them nothing to do with it.
 *
 * Three decisions worth knowing before changing this.
 *
 * **No platform gate.** Whether a phone can be enumerated arrives from the backend as a
 * refusal, never from guessing at the udid or the platform field here. A hardcoded
 * "Android only" would be a guess that goes stale the moment the iOS route lands, and the
 * refusal already carries a readable reason.
 *
 * **Names and icons come from the phone or not at all.** The helper reads them off
 * `PackageManager`; adb cannot. A row with neither shows its package id, and the footnote says
 * why — it never invents a prettier name or a stand-in picture.
 *
 * All the wording and every emptiness distinction live in `installedAppsView`, which is pure
 * and tested without rendering. This file fetches, draws, and launches.
 */
export function InstalledApps({ udid, deviceName, launchable }: Props) {
  const [load, setLoad] = useState<InstalledAppsLoad>({ state: "loading" });
  const [showSystem, setShowSystem] = useState(false);
  const [filter, setFilter] = useState("");
  /// Bumped by the refresh button. A counter rather than a second fetch path, so the effect
  /// below stays the only place that calls the backend — two fetch paths is how a panel ends
  /// up showing one phone's apps while pointed at another.
  const [reloads, setReloads] = useState(0);
  const [launching, setLaunching] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    setLoad({ state: "loading" });
    void (async () => {
      try {
        const apps = await listInstalledApps(udid);
        if (live) setLoad({ state: "ready", apps });
      } catch (error) {
        // The message, not a category. A backend that cannot enumerate says which
        // capability it lacks, and that sentence is more use than "failed".
        // `describeError`: a command rejection is an object, and `String` of one is
        // "[object Object]".
        if (live) setLoad({ state: "failed", reason: describeError(error) });
      }
    })();
    return () => {
      live = false;
    };
  }, [udid, reloads]);

  const view = installedAppsView(load, showSystem, filter);
  const ready = load.state === "ready";

  const launch = async (bundleId: string, name: string) => {
    if (launching) return;
    setLaunching(bundleId);
    try {
      await launchDeviceApp(udid, bundleId);
      pushToast("ok", "Đã mở app", name);
    } catch (error) {
      toastError("Mở app thất bại", error);
    } finally {
      setLaunching(null);
    }
  };

  return (
    <section
      className={`installed-apps${launchable ? " is-launchable" : ""}`}
      aria-label={`Ứng dụng trên ${deviceName}`}
    >
      <div className="row installed-apps-head">
        <span className="installed-apps-title">App List</span>
        <button
          type="button"
          className="ghost"
          title="Đọc lại danh sách ứng dụng từ máy"
          aria-label="Làm mới danh sách ứng dụng"
          onClick={() => setReloads((count) => count + 1)}
        >
          <IconRefresh size={13} />
        </button>
      </div>

      {ready && (
        <>
          <div className="row installed-apps-controls">
            <input
              value={filter}
              onChange={(event) => setFilter(event.target.value)}
              placeholder="Lọc theo tên hoặc tên gói"
              aria-label="Lọc ứng dụng"
            />
            <label title="Ứng dụng cài sẵn theo máy hoặc ROM">
              <input
                type="checkbox"
                checked={showSystem}
                onChange={(event) => setShowSystem(event.target.checked)}
              />
              Hệ thống ({view.systemCount})
            </label>
          </div>
          <p className="hint">
            {view.userCount} ứng dụng đã cài. {installedAppsFootnote(view.rows)}
          </p>
        </>
      )}

      {view.notice && (
        <p className="hint" role={view.notice.kind === "refused" ? "alert" : undefined}>
          {view.notice.text}
        </p>
      )}

      {view.rows.length > 0 && (
        <ul className="installed-apps-list">
          {view.rows.map((app) => {
            const name = app.label ?? app.bundleId;
            /* The phone's own icon when it gave one, and a neutral square when it did not —
               never a stand-in picture, so "no icon" cannot read as "this app looks like a
               grey box". `alt=""` because the name is right beside it. */
            const icon = app.iconPngBase64 ? (
              <img
                className="installed-apps-icon"
                src={`data:image/png;base64,${app.iconPngBase64}`}
                alt=""
                width={24}
                height={24}
              />
            ) : (
              <span className="installed-apps-icon is-blank" aria-hidden="true" />
            );
            const label = app.label ? (
              <span className="installed-apps-name" title={app.bundleId}>
                {name}
              </span>
            ) : (
              <code className="installed-apps-name">{app.bundleId}</code>
            );
            return (
              <li key={`${app.kind}:${app.bundleId}`}>
                {launchable ? (
                  <button
                    type="button"
                    className="installed-apps-row"
                    title={`Mở ${name} — ${app.bundleId}`}
                    disabled={launching !== null}
                    onClick={() => void launch(app.bundleId, name)}
                  >
                    {icon}
                    {label}
                    {launching === app.bundleId && <span className="hint">…</span>}
                  </button>
                ) : (
                  <>
                    {icon}
                    {label}
                  </>
                )}
                {app.kind === "system" && <span className="chip plain">hệ thống</span>}
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
