import { useEffect, useState } from "react";
import { listInstalledApps } from "../api";
import {
  installedAppsFootnote,
  installedAppsView,
  type InstalledAppsLoad,
} from "../installedAppsView";

interface Props {
  udid: string;
  deviceName: string;
}

/**
 * What is installed on one phone.
 *
 * Two decisions worth knowing before changing this.
 *
 * **No platform gate.** Whether a phone can be enumerated arrives from the backend as a
 * refusal, never from guessing at the udid or the platform field here. A hardcoded
 * "Android only" would be a guess that goes stale the moment the iOS route lands, and
 * the refusal already carries a readable reason.
 *
 * **Bundle ids are the names.** On Android a human-readable label is not obtainable over
 * adb at any sane price, so `label` is null and the row shows the bundle id. The panel
 * says so once, plainly, instead of inventing prettier names.
 *
 * All the wording and every emptiness distinction live in `installedAppsView`, which is
 * pure and tested without rendering. This file only fetches and draws.
 */
export function InstalledApps({ udid, deviceName }: Props) {
  const [load, setLoad] = useState<InstalledAppsLoad>({ state: "loading" });
  const [showSystem, setShowSystem] = useState(false);
  const [filter, setFilter] = useState("");

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
        if (live) setLoad({ state: "failed", reason: String(error) });
      }
    })();
    return () => {
      live = false;
    };
  }, [udid]);

  const view = installedAppsView(load, showSystem, filter);
  const ready = load.state === "ready";

  return (
    <section className="installed-apps" aria-label={`Ứng dụng trên ${deviceName}`}>
      {ready && (
        <>
          <div className="row installed-apps-controls">
            <input
              value={filter}
              onChange={(event) => setFilter(event.target.value)}
              placeholder="Lọc theo tên gói"
              aria-label="Lọc ứng dụng"
            />
            <label title="Ứng dụng cài sẵn theo máy hoặc ROM">
              <input
                type="checkbox"
                checked={showSystem}
                onChange={(event) => setShowSystem(event.target.checked)}
              />
              Hiện app hệ thống ({view.systemCount})
            </label>
          </div>
          <p className="hint">
            {view.userCount} ứng dụng đã cài. {installedAppsFootnote()}
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
          {view.rows.map((app) => (
            <li key={`${app.kind}:${app.bundleId}`}>
              <code>{app.label ?? app.bundleId}</code>
              {app.kind === "system" && <span className="chip plain">hệ thống</span>}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
