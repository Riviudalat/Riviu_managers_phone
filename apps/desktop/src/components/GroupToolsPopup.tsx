import { useId, useMemo, useState } from "react";
import { Users } from "lucide-react";
import { WorkspaceTabs } from "./WorkspacePrimitives";
import { useModalFocus } from "./useModalFocus";
import { recordedSteps } from "../macroStore";
import type { DeviceInfo } from "../types";
import { targetsOf } from "../selectionTargets";
import { IconClose } from "./Icons";
import { FileDistributionTool } from "./groupTools/FileDistributionTool";
import { GpsTool } from "./groupTools/GpsTool";
import { MacroTool } from "./groupTools/MacroTool";
import { PeripheralsTool } from "./groupTools/PeripheralsTool";
import { QuickActionsTool } from "./groupTools/QuickActionsTool";
import { QuickReplyTool } from "./groupTools/QuickReplyTool";
import { RootTool } from "./groupTools/RootTool";
import { TextDistributionTool } from "./groupTools/TextDistributionTool";

interface Props {
  devices: DeviceInfo[];
  selected: string[];
  onClose: () => void;
  visible?: boolean;
  macroTargets?: string[];
  macroOnly?: boolean;
  onBeginMacro?: (targets: string[]) => void;
  restoreFocus?: () => HTMLElement | null;
}

type Tool = "text" | "files" | "reply" | "keys" | "macro" | "gps" | "root" | "peripherals";

const TOOLS: { id: Tool; label: string; description: string }[] = [
  { id: "text", label: "Phân phối văn bản", description: "Soạn nội dung và xem trước văn bản cho từng máy." },
  { id: "files", label: "Phân phối tệp", description: "Chọn tệp, kiểm tra phân công rồi gửi đến máy đích." },
  { id: "reply", label: "Câu trả lời nhanh", description: "Chuẩn bị câu trả lời để sử dụng trên các máy đích." },
  { id: "keys", label: "Thao tác nhanh", description: "Thực hiện cùng một thao tác trên các máy trong phạm vi." },
  { id: "macro", label: "Macro", description: "Ghi, quản lý và chạy lại chuỗi thao tác." },
  { id: "gps", label: "Vị trí (GPS)", description: "Thiết lập vị trí cho các máy trong phạm vi." },
  { id: "root", label: "Root / Máy mới", description: "Công cụ bảo trì và chuẩn bị máy." },
  { id: "peripherals", label: "Ngoại vi", description: "Cấu hình bàn phím, chuột và thiết bị ngoại vi." },
];

export function GroupToolsPopup({ devices, selected, onClose, visible = true, macroTargets, macroOnly = false, onBeginMacro, restoreFocus }: Props) {
  const panelId = useId();
  const macroNameId = useId();
  const dialogRef = useModalFocus<HTMLDivElement>(onClose, visible, {
    initialFocus: () => macroTargets !== undefined && recordedSteps().length > 0
      ? document.getElementById(macroNameId) : null,
    restoreFocus,
  });
  const [tool, setTool] = useState<Tool>("text");
  const activeId = macroOnly ? "macro" : tool;
  const isMacroSession = activeId === "macro" && macroTargets !== undefined;
  const currentTargets = useMemo(() => targetsOf(selected, devices), [selected, devices]);
  // An explicitly empty recording scope must remain empty after roster changes.
  const targets = isMacroSession ? macroTargets : currentTargets;
  const targetDevices = useMemo(
    () =>
      targets
        .map((udid) => devices.find((d) => d.udid === udid))
        .filter((d): d is DeviceInfo => Boolean(d)),
    [targets, devices],
  );
  const scopeLabel = isMacroSession ? `${targets.length} máy trong phiên ghi`
    : selected.length ? `${targets.length} máy đã chọn` : `Tất cả ${targets.length} máy`;
  const activeTool = TOOLS.find((item) => item.id === activeId)!;

  return (
    <div className="modal-backdrop group-tools-layer" hidden={!visible} inert={!visible} aria-hidden={!visible ? true : undefined} onClick={onClose}>
      <div ref={dialogRef} tabIndex={-1} className="nurture-float group-tools" role="dialog" aria-modal="true" aria-label="Công cụ nhóm" onClick={(event) => event.stopPropagation()}>
        <div className="nurture-float-title">
          <strong>Công cụ nhóm</strong>
          <div className="grow" />
          <button type="button" className="close" title="Đóng" aria-label="Đóng công cụ nhóm" onClick={onClose}>
            <IconClose size={14} />
          </button>
        </div>

        <div className="nurture-float-body">
          <div className="group-tools-scope">
            <Users size={18} aria-hidden="true" />
            <div><strong>{scopeLabel}</strong><span>{isMacroSession ? "Phạm vi đã chốt khi bắt đầu ghi; đổi lựa chọn không thay máy đích." : selected.length ? "Thao tác áp dụng cho lựa chọn hiện tại." : "Chưa chọn riêng máy nào; thao tác áp dụng cho toàn bộ danh sách."}</span>
              {isMacroSession && <span>{targets.length ? targets.map(udid => devices.find(device => device.udid === udid)?.name ?? udid).join(", ") : "Chưa có máy đích; lưu Macro để dùng sau."}</span>}
            </div>
          </div>
          <WorkspaceTabs label="Công cụ nhóm" tabs={(macroOnly ? TOOLS.filter(item => item.id === "macro") : TOOLS).map((item) => ({ ...item, panelId }))} value={activeId} onChange={(value) => setTool(value as Tool)} />
          <section id={panelId} role="tabpanel" aria-label={activeTool.label} className="group-tools-panel">
            <div className="group-tools-intro"><h3>{activeTool.label}</h3><p>{activeTool.description}</p></div>
          {activeId === "text" && (
            <TextDistributionTool devices={devices} targets={targets} targetDevices={targetDevices} />
          )}
          {activeId === "files" && (
            <FileDistributionTool devices={devices} targets={targets} targetDevices={targetDevices} />
          )}
          {activeId === "reply" && <QuickReplyTool targets={targets} scopeLabel={scopeLabel} />}
          {activeId === "keys" && <QuickActionsTool targets={targets} scopeLabel={scopeLabel} />}
          <div hidden={activeId !== "macro"}>
            {(activeId === "macro" || macroTargets !== undefined) && <MacroTool targets={macroTargets ?? currentTargets}
              scopeLabel={macroTargets !== undefined ? `${macroTargets.length} máy trong phiên ghi` : scopeLabel}
              nameInputId={macroNameId} onStartRecording={() => onBeginMacro?.([...(macroTargets ?? currentTargets)])} />}
          </div>
          {activeId === "gps" && <GpsTool targets={targets} scopeLabel={scopeLabel} />}
          {activeId === "root" && <RootTool targets={targets} scopeLabel={scopeLabel} />}
          {activeId === "peripherals" && <PeripheralsTool targets={targets} scopeLabel={scopeLabel} />}
          </section>
        </div>
      </div>
    </div>
  );
}
