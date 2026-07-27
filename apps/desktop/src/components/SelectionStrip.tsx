import type { DeviceInfo } from "../types";

/** Shared selection strip so Publish/Apps/Jobs/etc. are usable without mystery disabled buttons. */
export function SelectionStrip({
  devices,
  selected,
  onSelectAll,
  onClear,
}: {
  devices: DeviceInfo[];
  selected: string[];
  onSelectAll: () => void;
  onClear: () => void;
}) {
  const n = selected.length || devices.length;
  const usingAll = selected.length === 0 && devices.length > 0;
  return (
    <div className="selection-strip">
      <span>
        {usingAll ? (
          <>
            Chưa chọn → sẽ dùng <strong>tất cả {devices.length}</strong> máy
          </>
        ) : (
          <>
            Đang chọn <strong>{selected.length}</strong> / {devices.length} máy
          </>
        )}
      </span>
      <div className="grow" />
      <button type="button" className="ghost" disabled={!devices.length} onClick={onSelectAll}>
        Chọn tất cả
      </button>
      <button type="button" className="ghost" disabled={!selected.length} onClick={onClear}>
        Bỏ chọn
      </button>
      {!devices.length && <span className="hint">Chưa có thiết bị — về Quản lý cửa sổ → Refresh</span>}
      {!!devices.length && (
        <span className="hint mono" title={selected.join(", ") || devices.map((d) => d.udid).join(", ")}>
          target×{n}
        </span>
      )}
    </div>
  );
}

export function targetsOf(selected: string[], devices: DeviceInfo[]): string[] {
  return selected.length ? selected : devices.map((d) => d.udid);
}

export function flash(msg: string) {
  window.alert(msg);
}
