import {
  Grid2X2,
  Monitor,
  SlidersHorizontal,
  FolderOpen,
  Usb,
  Wifi,
  Pin,
  PinOff,
  ChevronRight,
  Plus,
} from "lucide-react";
import { FOCUS_ZOOM, TILE_ZOOM, loadZoom, storeZoom } from "../zoom";
import type { ConnectionKind } from "../types";
import { useEffect, useRef, useState } from "react";
import { ControlStreamSettings } from "./ControlStreamSettings";

interface Props {
  tileWidth: number;
  onTileWidth: (n: number) => void;
  connection: "all" | "usb" | "wifi";
  onConnection: (n: "all" | "usb" | "wifi") => void;
  groups: { id: string; label: string; count: number; udids?: string[] }[];
  group: string;
  onGroup: (id: string) => void;
  machines: { id: string; number: number; name: string; selected: boolean; connection: ConnectionKind }[];
  onSelect: (id: string) => void;
  onSettings: () => void;
  onGroups?: () => void;
  onRotate?: () => void;
  pinned: boolean;
  onPinnedChange: (pinned: boolean) => void;
}
export function ControlCenterRail(p: Props) {
  const [focusWidth, setFocusWidth] = useState(() => loadZoom(FOCUS_ZOOM));
  const [peek, setPeek] = useState(false);
  const [expandedGroup, setExpandedGroup] = useState<string | null>(null);
  const leaveTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const clearLeave = () => { if (leaveTimer.current) clearTimeout(leaveTimer.current); };
  const reveal = () => { clearLeave(); setPeek(true); };
  const conceal = () => {
    clearLeave();
    leaveTimer.current = setTimeout(() => {
      const active = document.activeElement;
      if (panelRef.current?.querySelector("select:open") ||
        (active instanceof HTMLElement && panelRef.current?.contains(active) && active.matches(":focus-visible"))) return;
      setPeek(false);
    }, 240);
  };
  useEffect(() => () => { if (leaveTimer.current) clearTimeout(leaveTimer.current); }, []);
  const open = p.pinned || peek;
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
    <aside
      className={`control-center-rail rail-hover${p.pinned ? " is-pinned" : " is-unpinned"}${open ? " is-open" : ""}`}
      aria-label="Thiết lập Control Center"
      onPointerEnter={reveal}
      onPointerLeave={conceal}
      onFocusCapture={reveal}
      onBlurCapture={event => { if (!event.currentTarget.contains(event.relatedTarget)) conceal(); }}
      onKeyDown={event => {
        if (event.key === "Escape" && !p.pinned) {
          event.stopPropagation();
          event.currentTarget.querySelector<HTMLButtonElement>(".rail-handle")?.focus();
          setPeek(false);
        }
      }}
    >
      {!p.pinned && <button type="button" className="rail-handle" aria-label="Hiện bảng Hiển thị" aria-expanded={open} onClick={reveal}><SlidersHorizontal size={16}/><span>Hiển thị</span></button>}
      <div ref={panelRef} className="rail-panel" inert={!open} aria-hidden={!open}>
      <header>
        <SlidersHorizontal size={16} /><strong>Hiển thị</strong>
        <button type="button" className="rail-toggle" title={p.pinned ? "Bỏ ghim để tự ẩn" : "Ghim bảng Hiển thị"} aria-label={p.pinned ? "Bỏ ghim bảng Hiển thị" : "Ghim bảng Hiển thị"} aria-pressed={p.pinned} onClick={() => p.onPinnedChange(!p.pinned)}>{p.pinned ? <PinOff size={16}/> : <Pin size={16}/>}</button>
      </header>
      <div className="control-rail-content">
      <label>
        <span>
          <Monitor size={15} />
          Màn hình lớn <output>{focusWidth}px</output>
        </span>
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
      <label>
        <span>
          <Grid2X2 size={15} />Ô xem trước <output>{p.tileWidth}px</output>
        </span>
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
      <ControlStreamSettings />
      <div className="control-shortcuts"><button type="button" onClick={p.onSettings}><SlidersHorizontal size={15}/>Cài đặt</button><button type="button" onClick={p.onGroups}><Monitor size={15}/>Thiết bị</button><button type="button" onClick={p.onRotate}><Grid2X2 size={15}/>Xoay</button></div>
      <div
        className="control-connections"
        role="group"
        aria-label="Lọc kết nối"
      >
        {(["all", "usb", "wifi"] as const).map((c) => (
          <button
            type="button"
            key={c}
            aria-label={`${c === "all" ? "Tất cả" : c.toUpperCase()} · ${connectionCounts[c]} máy`}
            aria-pressed={p.connection === c}
            onClick={() => p.onConnection(c)}
          >
            {c === "usb" ? (
              <Usb size={14} />
            ) : c === "wifi" ? (
              <Wifi size={14} />
            ) : null}
            {c === "all" ? "Tất cả" : c.toUpperCase()}
            <small className="connection-count" aria-hidden="true">{connectionCounts[c]}</small>
          </button>
        ))}
      </div>
      <header>
        <FolderOpen size={16} />
        <strong>Nhóm thiết bị</strong>
        <button type="button" className="rail-create-group" title="Tạo nhóm" aria-label="Tạo nhóm thiết bị" onClick={p.onGroups}><Plus size={17}/></button>
      </header>
      <div className="control-group-list">
        {p.groups.map((g) => {
          const members = p.machines.filter(machine => !g.udids || g.udids.includes(machine.id));
          const expanded = expandedGroup === g.id;
          return <div key={g.id} className="rail-group">
          <button type="button" className="rail-group-trigger" aria-pressed={g.id === p.group} aria-expanded={expanded}
            onClick={() => { p.onGroup(g.id); setExpandedGroup(expanded ? null : g.id); }}>
            <ChevronRight size={15}/><span>{g.label}</span><small>{members.filter(m => m.selected).length} / {g.count}</small>
          </button>
          <div className="rail-group-expansion" data-open={expanded} inert={!expanded} aria-hidden={!expanded}><div>
          <div className="control-machine-grid" aria-label={`Chọn nhanh thiết bị · ${g.label}`}>
        {members.map((m) => (
          <button
            key={m.id}
            type="button"
            aria-label={`Chọn nhanh Máy ${m.number} · ${m.name}`}
            title={`Máy ${m.number} · ${m.name}`}
            aria-pressed={m.selected}
            onClick={() => p.onSelect(m.id)}
          >
            {m.number}
          </button>
        ))}
          </div></div></div>
          </div>;
        })}
      </div>
      </div>
      </div>
    </aside>
  );
}
