import { TargetSelector } from "./TargetSelector";
import { NurturePopup } from "./NurturePopup";
import { InteractionPopup } from "./InteractionPopup";
import { PublishPage } from "../pages/PublishPage";
import type { DeviceGroup, DeviceInfo, DeviceMeta, TargetRef } from "../types";
import type { DeviceAutomation } from "../features/devices/deviceAutomation";
import type { OperationSourceRef } from "../operationSource";
import { useState } from "react";

export function AutomationWorkspace({ kind, devices, groups, selected, targetRef, targetUdids,
  metas, labels, onTargetRefChange, onSelectUdids, operationSource, docked = false }: {
  kind: DeviceAutomation;
  devices: DeviceInfo[];
  groups: DeviceGroup[];
  selected: string[];
  targetRef: TargetRef;
  targetUdids: string[];
  metas: Map<string, DeviceMeta>;
  labels: Map<string, string>;
  onTargetRefChange: (target: TargetRef) => void;
  onSelectUdids: (udids: string[]) => void;
  operationSource?: OperationSourceRef;
  docked?: boolean;
}) {
  const [scopeExpanded, setScopeExpanded] = useState(false);
  const common = { devices, selected, targetRef, targetUdids, metas, onTargetRefChange, operationSource };
  return <div className="automation-page-stack">
    <details className={`automation-scope${docked ? " is-collapsible" : ""}`} open={!docked || scopeExpanded}>
      <summary hidden={!docked} onClick={(event) => { event.preventDefault(); setScopeExpanded((expanded) => !expanded); }}>
        Phạm vi thiết bị <strong>{targetRef.type === "all" ? "Toàn bộ · " : targetRef.type === "group"
          ? `${groups.find((group) => group.id === targetRef.groupId)?.name ?? "Nhóm"} · ` : ""}{targetUdids.length} máy</strong>
      </summary>
    <TargetSelector devices={devices} groups={groups} selected={selected} onChange={onSelectUdids}
      targetRef={targetRef} requireChoice onTargetRefChange={onTargetRefChange}
      deviceLabel={(device) => labels.get(device.udid) ?? device.name} />
    </details>
    {kind === "nurture" && <NurturePopup {...common} surface="page" />}
    {kind === "interaction" && <InteractionPopup {...common} surface="page" />}
    {kind === "publish" && <PublishPage {...common} onSelectUdids={onSelectUdids} />}
  </div>;
}
