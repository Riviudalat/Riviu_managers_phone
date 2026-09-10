import { NurturePopup } from "./NurturePopup";
import { InteractionPopup } from "./InteractionPopup";
import { PublishPage } from "../pages/PublishPage";
import type { DeviceGroup, DeviceInfo, DeviceMeta, TargetRef } from "../types";
import type { DeviceAutomation } from "../features/devices/deviceAutomation";
import type { OperationSourceRef } from "../operationSource";
import "../styles/automation-tabs.css";

export function AutomationWorkspace({ kind, devices, groups, selected, targetRef, targetUdids,
  metas, onTargetRefChange, onSelectUdids, operationSource }: {
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
  const common = { devices, selected, targetRef, targetUdids, metas, onTargetRefChange, operationSource };
  const scopeControl = <select className="machine-scope-select" aria-label="Phạm vi thiết bị"
    value={targetRef.type === "group" ? `group:${targetRef.groupId}` : targetRef.type}
    onChange={event => {
      const value = event.target.value;
      onTargetRefChange(value === "all" ? { type: "all" } : value.startsWith("group:")
        ? { type: "group", groupId: value.slice(6) } : { type: "explicit", udids: targetUdids });
    }}>
    <option value="explicit">{targetUdids.length ? `${targetUdids.length} máy đã chọn` : "Chọn từng máy"}</option><option value="all">Toàn bộ máy</option>
    {groups.map(group => <option key={group.id} value={`group:${group.id}`}>{group.name}</option>)}
  </select>;
  return <div className={`automation-page-stack${kind === "publish" ? " is-publish-workspace" : ""}`}
    style={kind === "publish" ? undefined : { gridTemplateRows: "minmax(0, 1fr)" }}>
    {kind === "nurture" && <NurturePopup {...common} surface="page" scopeControl={scopeControl} />}
    {kind === "interaction" && <InteractionPopup {...common} surface="page" scopeControl={scopeControl} />}
    {kind === "publish" && <PublishPage {...common} onSelectUdids={onSelectUdids} scopeControl={scopeControl} />}
  </div>;
}
