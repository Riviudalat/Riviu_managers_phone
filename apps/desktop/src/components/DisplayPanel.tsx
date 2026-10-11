import { ChevronsLeft, ChevronsRight, Monitor, Plus, RotateCw, Settings, Smartphone, Usb, Wifi } from "lucide-react";
import { FOCUS_ZOOM, TILE_ZOOM, loadZoom, storeZoom } from "../zoom";
import type { ConnectionKind } from "../types";
import { useState } from "react";
import { ControlStreamSettings } from "./ControlStreamSettings";

interface Props {
  /** Folded to its «/» rail: kept mounted (and its pending apply alive) but hidden and inert. */
  collapsed: boolean;
  onCollapsed: (collapsed: boolean) => void;
  tileWidth: number;
  onTileWidth: (n: number) => void;
  connection: "all" | "usb" | "wifi";
  onConnection: (n: "all" | "usb" | "wifi") => void;
  /** The group tabs, with how many of each group's phones are selected. */
  groups: { id: string; label: string; count: number; selected?: number; color?: string | null; udids?: string[] }[];
  group: string;
  onGroup: (id: string) => void;
  onCreateGroup: () => void;
  machines: { id: string; connection: ConnectionKind }[];
  onSettings: () => void;
  onDeviceSettings: () => void;
  onRotate?: () => void;
}

/**
 * The left "Cài đặt" panel of the Control Center (GF1), copied from GenFarmer's: wall and large
 * view sizes, stream quality and FPS, shortcuts, the connection filter and the group list with
 * "(chọn / tổng)" counts. It sits between the sidebar and the wall and folds to a narrow rail;
 * the fold is remembered.
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
    <aside className="cc-side-panel" data-collapsed={p.collapsed ? "true" : "false"}>
      <div className="cc-side-head">
        {!p.collapsed && <span className="cc-side-tab">Cài đặt</span>}
        <button
          type="button"
          className="ghost cc-side-toggle"
          aria-label="Bảng Cài đặt"
          title={p.collapsed ? "Mở bảng Cài đặt" : "Thu gọn bảng Cài đặt"}
          aria-expanded={!p.collapsed}
          aria-controls="control-display-panel"
          onClick={() => p.onCollapsed(!p.collapsed)}
        >
          {p.collapsed ? <ChevronsRight size={15} aria-hidden="true" /> : <ChevronsLeft size={15} aria-hidden="true" />}
        </button>
      </div>
      <section id="control-display-panel" className="cc-side-body" aria-label="Cài đặt hiển thị" hidden={p.collapsed} inert={p.collapsed}>
        <label className="cc-side-slider">
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
        <label className="cc-side-slider">
          <span><Smartphone size={14} aria-hidden="true" />Màn hình nhỏ <output>{p.tileWidth}px</output></span>
          <input
            aria-label="Kích thước ô xem trước"
            type="range"
            min={TILE_ZOOM.min}
            max={TILE_ZOOM.max}
            step={2}
            value={p.tileWidth}
            onChange={(e) => p.onTileWidth(Number(e.target.value))}
          />
        </label>
        <div className="cc-side-stream" role="group" aria-label="Luồng hình · áp dụng cho mọi máy Android">
          <ControlStreamSettings />
        </div>
        <div className="cc-side-shortcuts">
          <button type="button" onClick={p.onSettings}><Settings size={14} aria-hidden="true" />Cài đặt</button>
          <button type="button" onClick={p.onDeviceSettings}><Smartphone size={14} aria-hidden="true" />Thiết bị</button>
          <button type="button" onClick={p.onRotate} title="Xoay máy Android đã chọn"><RotateCw size={14} aria-hidden="true" />Xoay</button>
        </div>
        <div className="control-connections cc-side-connections" role="group" aria-label="Lọc kết nối">
          {(["all", "usb", "wifi"] as const).map((c) => (
            <button
              type="button"
              key={c}
              aria-label={`${c === "all" ? "Tất cả" : c.toUpperCase()} · ${connectionCounts[c]} máy`}
              aria-pressed={p.connection === c}
              onClick={() => p.onConnection(c)}
            >
              {c === "usb" ? <Usb size={13} aria-hidden="true" /> : c === "wifi" ? <Wifi size={13} aria-hidden="true" /> : null}
              {c === "all" ? "Tất cả" : c === "usb" ? "USB" : "Wi‑Fi"}
              <small className="connection-count" aria-hidden="true">{connectionCounts[c]}</small>
            </button>
          ))}
        </div>
        <div className="cc-side-groups">
          <div className="cc-side-groups-head">
            <span>Nhóm</span>
            <button type="button" className="ghost cc-side-add" aria-label="Tạo nhóm thiết bị" title="Tạo nhóm (box) thiết bị" onClick={p.onCreateGroup}>
              <Plus size={14} aria-hidden="true" />
            </button>
          </div>
          <div className="cc-side-group-list" role="tablist" aria-label="Nhóm thiết bị" aria-orientation="vertical">
            {p.groups.map((group) => (
              <button
                type="button"
                key={group.id}
                role="tab"
                aria-selected={group.id === p.group}
                className={group.id === p.group ? "active" : ""}
                onClick={() => p.onGroup(group.id)}
              >
                {group.color && <span className="group-tabs-dot" style={{ background: group.color }} aria-hidden="true" />}
                <span className="cc-side-group-name">{group.label}</span>
                <span className="cc-side-group-count" aria-label={`${group.selected ?? 0} đã chọn trên ${group.count} máy`}>({group.selected ?? 0} / {group.count})</span>
              </button>
            ))}
          </div>
        </div>
      </section>
    </aside>
  );
}
