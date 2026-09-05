import { useId, useMemo, useState } from "react";
import { CheckCheck, Search, X } from "lucide-react";

import type { DeviceGroup, DeviceInfo, TargetRef } from "../types";
import "./TargetSelector.css";

export type TargetSelectorMode = "all" | "group" | "explicit";

export interface TargetSelectorProps {
  devices: DeviceInfo[];
  groups: DeviceGroup[];
  /** An empty selection deliberately means every device in the current roster. */
  selected: string[];
  onChange: (udids: string[]) => void;
  /** Controlled semantic scope. Unlike `selected`, an empty Group never means All. */
  targetRef?: TargetRef;
  onTargetRefChange?: (target: TargetRef) => void;
  deviceLabel?: (device: DeviceInfo, index: number) => string;
  label?: string;
  requireChoice?: boolean;
}

function defaultDeviceLabel(device: DeviceInfo, index: number): string {
  return `Máy ${index + 1} · ${device.name}`;
}

/**
 * Shared fleet target control. It receives groups with the roster so choosing a group always
 * resolves its members at the moment of the click, never from an older screen snapshot.
 */
export function TargetSelector({
  devices,
  groups,
  selected,
  onChange,
  targetRef,
  onTargetRefChange,
  deviceLabel = defaultDeviceLabel,
  label = "Phạm vi thiết bị",
  requireChoice = false,
}: TargetSelectorProps) {
  const baseId = useId();
  const [internalMode, setInternalMode] = useState<TargetSelectorMode>(
    selected.length ? "explicit" : "all",
  );
  const [internalGroupId, setInternalGroupId] = useState("");
  const [choiceMade, setChoiceMade] = useState(false);
  const [search, setSearch] = useState("");
  const emptyTarget = targetRef?.type === "explicit" && targetRef.udids.length === 0;
  const mode = requireChoice && !choiceMade && emptyTarget ? null : targetRef?.type ?? internalMode;
  const groupId = targetRef?.type === "group" ? targetRef.groupId : internalGroupId;

  const rosterIds = useMemo(() => new Set(devices.map((device) => device.udid)), [devices]);
  const selectedIds = useMemo(
    () => new Set(
      (targetRef?.type === "explicit" ? targetRef.udids : selected)
        .filter((udid) => rosterIds.has(udid)),
    ),
    [rosterIds, selected, targetRef],
  );
  const groupOptions = useMemo(
    () =>
      groups.map((group) => ({
        group,
        udids: devices.filter((device) => group.udids.includes(device.udid)).map((device) => device.udid),
      })),
    [devices, groups],
  );
  const activeGroup = groupOptions.find((entry) => entry.group.id === groupId);
  const explicitIds = selectedIds;
  const visibleDevices = devices.map((device, index) => ({ device, index }))
    .filter(({ device, index }) => `${deviceLabel(device, index)} ${device.udid}`
      .toLocaleLowerCase("vi").includes(search.trim().toLocaleLowerCase("vi")));
  const updateExplicit = (ids: Set<string>) => {
    const udids = devices.filter((device) => ids.has(device.udid)).map((device) => device.udid);
    onChange(udids);
    onTargetRefChange?.({ type: "explicit", udids });
  };

  const summary =
    mode === null ? "Chưa chọn phạm vi" : mode === "all"
      ? `Toàn bộ ${devices.length}`
      : mode === "group"
        ? (activeGroup ? `${activeGroup.group.name} · ${activeGroup.udids.length} máy` : "Chọn một nhóm")
        : `${explicitIds.size} máy cụ thể`;

  const selectMode = (next: TargetSelectorMode) => {
    setChoiceMade(true);
    setInternalMode(next);
    if (next === "all") {
      setInternalGroupId("");
      onChange([]);
      onTargetRefChange?.({ type: "all" });
      return;
    }
    if (next === "explicit") {
      setInternalGroupId("");
      const udids = devices.filter((device) => selectedIds.has(device.udid)).map((device) => device.udid);
      onChange(udids);
      onTargetRefChange?.({ type: "explicit", udids });
      return;
    }
    if (next === "group") {
      const nextGroup = activeGroup ?? groupOptions[0];
      if (!nextGroup) return;
      setInternalGroupId(nextGroup.group.id);
      onChange(nextGroup.udids);
      onTargetRefChange?.({ type: "group", groupId: nextGroup.group.id });
    }
  };

  return (
    <fieldset className="target-selector" aria-labelledby={`${baseId}-label`}>
      <div className="target-selector-heading">
        <span id={`${baseId}-label`} className="target-selector-legend">
          {label}
        </span>
        <output role="status" aria-live="polite">
          {summary}
        </output>
      </div>

      <div className="target-selector-modes" role="radiogroup" aria-label="Cách chọn thiết bị">
        <label>
          <input
            type="radio"
            name={`${baseId}-mode`}
            checked={mode === "all"}
            onChange={() => selectMode("all")}
          />
          <span>Toàn bộ</span>
        </label>
        <label>
          <input
            type="radio"
            name={`${baseId}-mode`}
            checked={mode === "group"}
            disabled={groupOptions.length === 0}
            onChange={() => selectMode("group")}
          />
          <span>Nhóm</span>
        </label>
        <label>
          <input
            type="radio"
            name={`${baseId}-mode`}
            checked={mode === "explicit"}
            disabled={devices.length === 0}
            onChange={() => selectMode("explicit")}
          />
          <span>Máy cụ thể</span>
        </label>
      </div>

      {devices.length === 0 && <p className="target-selector-empty">Chưa có thiết bị phù hợp.</p>}

      {mode === "group" && devices.length > 0 && (
        <label className="target-selector-group" htmlFor={`${baseId}-group`}>
          <span>Nhóm thiết bị</span>
          <select
            id={`${baseId}-group`}
            aria-label="Chọn nhóm thiết bị"
            value={groupId}
            onChange={(event) => {
              const next = groupOptions.find((entry) => entry.group.id === event.currentTarget.value);
              if (!next) return;
              setInternalGroupId(next.group.id);
              onChange(next.udids);
              onTargetRefChange?.({ type: "group", groupId: next.group.id });
            }}
          >
            <option value="">Chọn một nhóm</option>
            {groupOptions.map(({ group, udids }) => (
              <option key={group.id} value={group.id}>
                {group.name} ({udids.length} máy)
              </option>
            ))}
          </select>
        </label>
      )}

      {mode === "explicit" && devices.length > 0 && (
        <div className="target-selector-explicit">
          <div className="target-selector-search">
            <label><Search size={16} /><input type="search" aria-label="Tìm máy trong phạm vi"
              placeholder="Tìm số máy, tên máy" value={search}
              onChange={(event) => setSearch(event.currentTarget.value)} /></label>
            <button type="button" className="ghost" onClick={() => updateExplicit(new Set([
              ...explicitIds, ...visibleDevices.map(({ device }) => device.udid),
            ]))}><CheckCheck size={16} /> Chọn đang hiện</button>
            <button type="button" className="icon-btn" title="Bỏ chọn máy" aria-label="Bỏ chọn máy"
              onClick={() => updateExplicit(new Set())}><X size={16} /></button>
          </div>
        <div
          className="target-selector-devices"
          role="group"
          aria-label="Danh sách máy cụ thể"
        >
          {visibleDevices.map(({ device, index }) => {
            const checked = explicitIds.has(device.udid);
            return (
              <label key={device.udid} htmlFor={`${baseId}-device-${index}`}>
                <input
                  id={`${baseId}-device-${index}`}
                  type="checkbox"
                  checked={checked}
                  onChange={(event) => {
                    const next = new Set(explicitIds);
                    if (event.currentTarget.checked) next.add(device.udid);
                    else next.delete(device.udid);
                    updateExplicit(next);
                  }}
                />
                <span>{deviceLabel(device, index)}</span>
              </label>
            );
          })}
        </div>
        {visibleDevices.length === 0 && <p className="target-selector-empty">Không có máy khớp tìm kiếm.</p>}
        </div>
      )}
    </fieldset>
  );
}
