import { useCallback, useEffect, useMemo, useState } from "react";
import { deviceDeletePath, deviceListDir, devicePullPath, devicePushFile } from "../api";
import { requestConfirm } from "../confirmStore";
import { describeError, pushToast, toastError } from "../toastStore";
import { pickDirectory, pickFile } from "../pickFile";
import {
  deviceCrumbs,
  DEVICE_HOME,
  formatDeviceSize,
  isBrowsableEntry,
  joinDevicePath,
  parentDevicePath,
  sortDeviceEntries,
} from "../deviceFiles";
import type { DeviceFileEntry, DeviceInfo } from "../types";

interface Props {
  device: DeviceInfo;
  onClose: () => void;
}

/**
 * Where an operator actually goes, so the common cases are one click and not five.
 *
 * Shortcuts are a convenience and **not** the reach of this browser: the typed path bar below
 * goes anywhere the phone will list, `/` included. A browser whose only destinations are five
 * buttons is a browser that cannot answer "what is in /data/local/tmp".
 */
const SHORTCUTS: { label: string; path: string }[] = [
  { label: "Bộ nhớ máy", path: DEVICE_HOME },
  { label: "Tải về", path: "/sdcard/Download" },
  { label: "Ảnh chụp", path: "/sdcard/DCIM/Camera" },
  { label: "Ảnh Riviu", path: "/sdcard/Pictures" },
  { label: "Phim", path: "/sdcard/Movies" },
  { label: "Android/data", path: "/sdcard/Android/data" },
  { label: "Gốc /", path: "/" },
];

/**
 * The phone's own filesystem, browsable (xiaowei "Preview Mobile Files").
 *
 * The app already had two file paths to a phone and neither of them is this one. Import and
 * export media know about the *gallery*: they stage a campaign, tell MediaStore about the
 * result, and deal only in pictures and videos. This is the file manager — any file, any
 * directory, and no claim beyond "these are the bytes that are there".
 *
 * Three things it refuses to fake, all learned from the rules in AGENTS.md §7:
 *
 * - **An unreadable directory says so.** A phone answers `ls` on a path it cannot read with
 *   exit 1 and a sentence on stderr; rendering that as an empty folder would claim the
 *   directory exists and is empty, which is a different fact.
 * - **A delete is read back.** `rm -rf` is silent about what it could not remove, so the
 *   backend lists the path afterwards and fails if it is still there.
 * - **Nothing is selected across a navigation.** Keeping a selection while the listing
 *   changes underneath is how a delete lands on the wrong file — the names are the same in
 *   twenty folders.
 */
export function DeviceFilesPopup({ device, onClose }: Props) {
  const [path, setPath] = useState(DEVICE_HOME);
  const [entries, setEntries] = useState<DeviceFileEntry[] | null>(null);
  const [failed, setFailed] = useState<string | null>(null);
  const [picked, setPicked] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  /// What is in the path box, which is not the same as where the browser *is*: the operator
  /// types freely and only Enter commits. Kept in step with `path` on every navigation, so
  /// clicking a folder updates the box too.
  const [typed, setTyped] = useState(DEVICE_HOME);

  const load = useCallback(async (target: string) => {
    setEntries(null);
    setFailed(null);
    // Cleared here rather than in the click handler, so every route into a new listing —
    // a crumb, a shortcut, Up, a refresh after a delete — drops the selection.
    setPicked([]);
    try {
      setEntries(await deviceListDir(device.udid, target));
    } catch (error) {
      // `describeError` and not `String(error)`: a Tauri command rejects with a plain
      // object (`{code, message}`), and `String({})` is the literally useless
      // "[object Object]" — which is what this panel showed for a folder the phone
      // refused, and why "it cannot reach the phone's folders" was a fair reading.
      setFailed(describeError(error));
    }
  }, [device.udid]);

  useEffect(() => {
    setTyped(path);
    void load(path);
  }, [load, path]);

  /// Go where the box says. Absolute-only, checked here so the operator gets the reason
  /// immediately rather than after a round trip — the backend validates it again regardless,
  /// and its validator is the one that matters.
  const goTyped = () => {
    const target = typed.trim();
    if (!target) return;
    if (!target.startsWith("/")) {
      setFailed("Đường dẫn phải bắt đầu bằng / — ví dụ /sdcard/Download.");
      return;
    }
    if (target === path) {
      void load(path);
      return;
    }
    setPath(target);
  };

  const rows = useMemo(() => sortDeviceEntries(entries ?? []), [entries]);
  const crumbs = deviceCrumbs(path);
  const up = parentDevicePath(path);

  const toggle = (name: string) => {
    setPicked((current) =>
      current.includes(name) ? current.filter((row) => row !== name) : [...current, name],
    );
  };

  const pullPicked = async () => {
    if (picked.length === 0 || busy) return;
    const dest = await pickDirectory("Lưu vào thư mục nào trên máy tính");
    if (!dest) return;
    setBusy(true);
    let saved = 0;
    const failures: string[] = [];
    for (const name of picked) {
      try {
        await devicePullPath(device.udid, joinDevicePath(path, name), dest);
        saved += 1;
      } catch (error) {
        // Per file, and the run continues: one unreadable file out of twenty must not
        // cancel the other nineteen, and the operator has to be told which one it was.
        failures.push(`${name}: ${describeError(error)}`);
      }
    }
    setBusy(false);
    if (saved > 0) pushToast("ok", `Đã lấy ${saved} mục về máy tính`, dest);
    if (failures.length > 0) {
      pushToast("warn", `${failures.length} mục không lấy được`, failures.join("\n"));
    }
  };

  const deletePicked = async () => {
    if (picked.length === 0 || busy) return;
    const ok = await requestConfirm({
      title: `Xoá ${picked.length} mục trên ${device.name}?`,
      // The names, not the count. A confirm that does not say what it is about to delete is
      // a confirm nobody can answer correctly.
      message: `${picked.join(", ")}\n\nXoá khỏi máy, không có thùng rác và không lấy lại được.`,
      confirmLabel: "Xoá",
      danger: true,
    });
    if (!ok) return;
    setBusy(true);
    let removed = 0;
    const failures: string[] = [];
    for (const name of picked) {
      try {
        await deviceDeletePath(device.udid, joinDevicePath(path, name));
        removed += 1;
      } catch (error) {
        failures.push(`${name}: ${describeError(error)}`);
      }
    }
    setBusy(false);
    if (removed > 0) pushToast("ok", `Đã xoá ${removed} mục`);
    if (failures.length > 0) {
      pushToast("warn", `${failures.length} mục không xoá được`, failures.join("\n"));
    }
    await load(path);
  };

  const pushHere = async () => {
    if (busy) return;
    const local = await pickFile({ title: "Chọn tệp đưa vào máy" });
    if (!local) return;
    setBusy(true);
    try {
      const landed = await devicePushFile(device.udid, local, path);
      pushToast("ok", "Đã đưa tệp vào máy", landed);
      await load(path);
    } catch (error) {
      toastError("Đưa tệp vào máy thất bại", error);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal device-files"
        role="dialog"
        aria-label={`Tệp trên ${device.name}`}
        onClick={(event) => event.stopPropagation()}
      >
        <header>
          <strong>
            Tệp trên máy — {device.name} <span className="muted">({device.udid})</span>
          </strong>
          <button type="button" className="ghost" onClick={onClose}>
            Đóng
          </button>
        </header>

        <div className="row device-files-jumps">
          {SHORTCUTS.map((shortcut) => (
            <button
              key={shortcut.path}
              type="button"
              className={`tb-btn ${path === shortcut.path ? "active" : ""}`}
              onClick={() => setPath(shortcut.path)}
            >
              {shortcut.label}
            </button>
          ))}
        </div>

        <div className="row device-files-bar">
          <button type="button" className="ghost" disabled={!up} onClick={() => up && setPath(up)}>
            ↑ Lên
          </button>
          <nav className="device-files-crumbs" aria-label="Các cấp thư mục">
            {crumbs.map((crumb, index) => (
              <span key={crumb.path}>
                {index > 0 && <span className="muted"> / </span>}
                <button type="button" className="link" onClick={() => setPath(crumb.path)}>
                  {crumb.label}
                </button>
              </span>
            ))}
          </nav>
          <button type="button" className="ghost" onClick={() => void load(path)}>
            Tải lại
          </button>
        </div>

        {/* Typed access, because breadcrumbs and shortcuts can only reach what is already on
            screen. Anything the phone will list is reachable from here: /data/local/tmp,
            /sdcard/Android/data, /system. A path the phone refuses answers with the phone's
            own sentence, which is the useful half of a refusal. */}
        <div className="row device-files-goto">
          <label htmlFor="device-files-path">Đường dẫn</label>
          <input
            id="device-files-path"
            value={typed}
            spellCheck={false}
            placeholder="/sdcard/Download"
            onChange={(event) => setTyped(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") goTyped();
            }}
          />
          <button type="button" className="ghost" onClick={goTyped}>
            Đi tới
          </button>
        </div>

        {entries === null && !failed && <p className="hint">Đang đọc {path} …</p>}
        {failed && (
          <p className="hint" role="alert">
            {failed}
          </p>
        )}

        {entries !== null && rows.length === 0 && !failed && (
          <p className="hint">Thư mục này rỗng.</p>
        )}

        {rows.length > 0 && (
          <ul className="device-files-list">
            {rows.map((entry) => (
              <li key={entry.name} className={picked.includes(entry.name) ? "is-picked" : ""}>
                <input
                  type="checkbox"
                  aria-label={`Chọn ${entry.name}`}
                  checked={picked.includes(entry.name)}
                  onChange={() => toggle(entry.name)}
                />
                {isBrowsableEntry(entry) ? (
                  <button
                    type="button"
                    className="link device-files-name"
                    onClick={() => setPath(joinDevicePath(path, entry.name))}
                  >
                    {entry.kind === "directory" ? "📁" : "🔗"} {entry.name}
                  </button>
                ) : (
                  <span className="device-files-name">📄 {entry.name}</span>
                )}
                <span className="device-files-size">{formatDeviceSize(entry)}</span>
                {/* The phone's own text, in the phone's own timezone. See `DeviceFileEntry`. */}
                <span className="device-files-when">{entry.modified ?? "—"}</span>
              </li>
            ))}
          </ul>
        )}

        <footer className="row device-files-actions">
          <span className="hint">
            {picked.length > 0 ? `Đã chọn ${picked.length} mục` : "Chưa chọn mục nào"}
          </span>
          <button type="button" className="ghost" disabled={busy} onClick={() => void pushHere()}>
            Đưa tệp vào đây
          </button>
          <button
            type="button"
            className="primary"
            disabled={busy || picked.length === 0}
            onClick={() => void pullPicked()}
          >
            Lấy về máy tính
          </button>
          <button
            type="button"
            className="danger"
            disabled={busy || picked.length === 0}
            onClick={() => void deletePicked()}
          >
            Xoá
          </button>
        </footer>
      </div>
    </div>
  );
}
