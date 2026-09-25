import { NurturePopup } from "./NurturePopup";
import { InteractionPopup } from "./InteractionPopup";
import { PublishPage } from "../pages/PublishPage";
import type { DeviceGroup, DeviceInfo, DeviceMeta, TargetRef } from "../types";
import type { DeviceAutomation } from "../features/devices/deviceAutomation";
import type { OperationSourceRef } from "../operationSource";
import "../styles/automation-tabs.css";
import { useState } from "react";
import { TargetSelector } from "./TargetSelector";
import { DetailDrawer } from "./WorkspacePrimitives";

export function AutomationWorkspace({ kind, devices, groups, selected, targetRef, targetUdids,
  metas, labels, onTargetRefChange, onSelectUdids, operationSource }: {
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
  const [scopeOpen, setScopeOpen] = useState(false);
  const common = { devices, selected, targetRef, targetUdids, metas, onTargetRefChange, operationSource };
  const scopeControl = <><select className="machine-scope-select" aria-label="Phạm vi thiết bị"
    value={targetRef.type === "group" ? `group:${targetRef.groupId}` : targetRef.type}
    onChange={event => {
      const value = event.target.value;
      onTargetRefChange(value === "all" ? { type: "all" } : value.startsWith("group:")
        ? { type: "group", groupId: value.slice(6) } : { type: "explicit", udids: targetUdids });
      if (value === "explicit") setScopeOpen(true);
    }}>
    <option value="explicit">{targetUdids.length ? `${targetUdids.length} máy đã chọn` : "Chọn từng máy"}</option><option value="all">Toàn bộ máy</option>
    {groups.map(group => <option key={group.id} value={`group:${group.id}`}>{group.name}</option>)}
  </select><button type="button" className="ghost machine-scope-pick" aria-label="Chọn thiết bị" aria-haspopup="dialog" aria-expanded={scopeOpen}
    onClick={() => { onTargetRefChange({ type: "explicit", udids: targetUdids }); setScopeOpen(true); }}><span className="scope-pick-full">Chọn thiết bị</span><span className="scope-pick-short" aria-hidden="true">Chọn</span></button></>;
  return <div className={`automation-page-stack${kind === "publish" ? " is-publish-workspace" : ""}`}
    style={kind === "publish" ? undefined : { gridTemplateRows: "minmax(0, 1fr)" }}>
    {kind === "nurture" && <NurturePopup {...common} surface="page" scopeControl={scopeControl} />}
    {kind === "interaction" && <InteractionPopup {...common} surface="page" scopeControl={scopeControl} />}
    {kind === "publish" && <PublishPage {...common} onSelectUdids={onSelectUdids} scopeControl={scopeControl} />}
    <DetailDrawer open={scopeOpen} title="Chọn thiết bị thực hiện" onClose={() => setScopeOpen(false)}
      footer={<button type="button" className="primary" onClick={() => setScopeOpen(false)}>Xong</button>}>
      <p>Chọn máy chỉ sửa phạm vi. Khi bắt đầu lượt mới, Riviu dừng tác vụ cũ liên quan và chờ nhả máy; kết quả đã gửi vẫn được giữ.</p>
      <TargetSelector devices={devices} groups={groups} selected={targetUdids} targetRef={targetRef}
        onChange={() => {}} onTargetRefChange={onTargetRefChange}
        deviceLabel={device => labels.get(device.udid) ?? device.name ?? device.udid} />
    </DetailDrawer>
  </div>;
}
