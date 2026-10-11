import { Grid2X2, Monitor, RotateCw, SlidersHorizontal, Usb, Wifi } from "lucide-react";
import { FOCUS_ZOOM, TILE_PRESETS, TILE_ZOOM, loadZoom, storeZoom, tileDensity } from "../zoom";
import type { ConnectionKind } from "../types";
import { useState } from "react";
import { ControlStreamSettings } from "./ControlStreamSettings";

interface Props {
  tileWidth: number;
  onTileWidth: (n: number) => void;
  connection: "all" | "usb" | "wifi";
  onConnection: (n: "all" | "usb" | "wifi") => void;
  groups: { id: string; label: string; count: number; udids?: string[] }[];
  group: string;
  machines: { id: string; connection: ConnectionKind }[];
  onSettings: () => void;
  onRotate?: () => void;
}

/**
 * The "Hiển thị" panel (PLAN §2): one horizontal band under the toolbar, opened by the toolbar
 * button and never on hover. It replaced the vertical rail, which took 208 px from every
 * laptop wall. Box choice lives in the toolbar's box chips now, not here.
 *
 * Stream settings say "Đã áp dụng" only once `set_stream_settings` has answered
 * (`ControlStreamSettings`), never on release of a slider.
 */
export function DisplayPanel(p: Props) {
  const [focusWidth, setFocusWidth] = useState(() => loadZoom(FOCUS_ZOOM));
  const activeGroup = p.groups.find(group => group.id === p.group);
  const members = activeGroup?.udids
    ? p.machines.filter(machine => activeGroup.udids?.includes(machine.id))
    : p.machines;
  const connectionCounts = {
    all: members.length,
    usb: members.filter(machine => machine.connection === "usb").length,
    wifi: members.filter(machine => machine.connection === "wifi").length,
  };
  return (
    <section id="control-display-panel" className="control-display-panel" aria-label="Hiển thị">
      <fieldset className="display-group">
        <legend><Grid2X2 size={15} aria-hidden="true" />Ô màn hình</legend>
        <div className="control-tile-presets" role="group" aria-label="Cỡ ô xem trước">
          {TILE_PRESETS.map((preset) => (
            <button
              type="button"
              key={preset.density}
              aria-pressed={tileDensity(p.tileWidth) === preset.density}
              title={`${preset.label} · ${preset.width}px`}
              onClick={() => p.onTileWidth(preset.width)}
            >
              {preset.label}
            </button>
          ))}
        </div>
        <label>
          <span>Cỡ ô <output>{p.tileWidth}px</output></span>
          <input
            aria-label="Kích thước ô xem trước"
            type="range"
            min={TILE_ZOOM.min}
            max={TILE_ZOOM.max}
            step={10}
            value={p.tileWidth}
            onChange={(e) => p.onTileWidth(Number(e.target.value))}
          />
        </label>
        <label>
          <span><Monitor size={14} aria-hidden="true" />Màn hình lớn <output>{focusWidth}px</output></span>
          <input
            aria-label="Kích thước màn hình điều khiển"
            type="range"
            min={FOCUS_ZOOM.min}
            max={FOCUS_ZOOM.max}
            step={10}
            value={focusWidth}
            onChange={(e) => {
              const v = Number(e.target.value);
              setFocusWidth(v);
              storeZoom(FOCUS_ZOOM, v);
            }}
          />
        </label>
      </fieldset>
      <fieldset className="display-group display-stream">
        <legend><SlidersHorizontal size={15} aria-hidden="true" />Luồng hình · áp dụng cho mọi máy Android</legend>
        <ControlStreamSettings />
      </fieldset>
      <fieldset className="display-group">
        <legend>Kết nối</legend>
        <div className="control-connections" role="group" aria-label="Lọc kết nối">
          {(["all", "usb", "wifi"] as const).map((c) => (
            <button
              type="button"
              key={c}
              aria-label={`${c === "all" ? "Tất cả" : c.toUpperCase()} · ${connectionCounts[c]} máy`}
              aria-pressed={p.connection === c}
              onClick={() => p.onConnection(c)}
            >
              {c === "usb" ? <Usb size={14} /> : c === "wifi" ? <Wifi size={14} /> : null}
              {c === "all" ? "Tất cả" : c.toUpperCase()}
              <small className="connection-count" aria-hidden="true">{connectionCounts[c]}</small>
            </button>
          ))}
        </div>
        <div className="control-shortcuts">
          <button type="button" onClick={p.onRotate}><RotateCw size={15} aria-hidden="true" />Xoay máy đã chọn</button>
          <button type="button" onClick={p.onSettings}><SlidersHorizontal size={15} aria-hidden="true" />Cài đặt</button>
        </div>
      </fieldset>
    </section>
  );
}
