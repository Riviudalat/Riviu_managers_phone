import { useMemo, useState } from "react";
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

/**
 * The group Tools popup: eight tools over the current selection, one at a time.
 *
 * 1,436 lines and no test of its own — the largest untested frontend unit in the repo. It
 * was already well factored inside: every tool is a component taking `{ targets, scopeLabel }`
 * and nothing else, so the file was eight independent things sharing a tab strip. This is
 * now the tab strip; each tool is its own file, and the pure identity generators are apart
 * from all of them.
 */
interface Props {
  devices: DeviceInfo[];
  selected: string[];
  onClose: () => void;
}

type Tool = "text" | "files" | "reply" | "keys" | "macro" | "gps" | "root" | "peripherals";

/**
 * Group Tools — batch operations scoped to the current selection (xiaowei device
 * context-menu tools). Text Distribution (A2) and Quick Replies (A6) so far; more tools dock
 * into the same tabbed panel as they land.
 */
export function GroupToolsPopup({ devices, selected, onClose }: Props) {
  const [tool, setTool] = useState<Tool>("text");
  const targets = useMemo(() => targetsOf(selected, devices), [selected, devices]);
  const targetDevices = useMemo(
    () =>
      targets
        .map((udid) => devices.find((d) => d.udid === udid))
        .filter((d): d is DeviceInfo => Boolean(d)),
    [targets, devices],
  );
  const scopeLabel = selected.length ? `${selected.length} máy` : `Tất cả ${devices.length}`;

  return (
    <div className="nurture-float-layer" aria-label="Công cụ nhóm">
      <div className="nurture-float group-tools">
        <div className="nurture-float-title" style={{ cursor: "default" }}>
          <strong>Công cụ nhóm</strong>
          <span className="hint">{scopeLabel}</span>
          <div className="grow" />
          <button type="button" className="close" title="Đóng" onClick={onClose}>
            <IconClose size={14} />
          </button>
        </div>

        <div className="nurture-float-body">
          <div className="group-tools-tabs">
            <button
              type="button"
              className={`tb-btn ${tool === "text" ? "active" : ""}`}
              onClick={() => setTool("text")}
            >
              Phân phối văn bản
            </button>
            <button
              type="button"
              className={`tb-btn ${tool === "files" ? "active" : ""}`}
              onClick={() => setTool("files")}
            >
              Phân phối tệp
            </button>
            <button
              type="button"
              className={`tb-btn ${tool === "reply" ? "active" : ""}`}
              onClick={() => setTool("reply")}
            >
              Câu trả lời nhanh
            </button>
            <button
              type="button"
              className={`tb-btn ${tool === "keys" ? "active" : ""}`}
              onClick={() => setTool("keys")}
            >
              Thao tác nhanh
            </button>
            <button
              type="button"
              className={`tb-btn ${tool === "macro" ? "active" : ""}`}
              onClick={() => setTool("macro")}
            >
              Macro
            </button>
            <button
              type="button"
              className={`tb-btn ${tool === "gps" ? "active" : ""}`}
              onClick={() => setTool("gps")}
            >
              Vị trí (GPS)
            </button>
            <button
              type="button"
              className={`tb-btn ${tool === "root" ? "active" : ""}`}
              onClick={() => setTool("root")}
            >
              Root / Máy mới
            </button>
            <button
              type="button"
              className={`tb-btn ${tool === "peripherals" ? "active" : ""}`}
              onClick={() => setTool("peripherals")}
            >
              Ngoại vi
            </button>
          </div>

          {tool === "text" && (
            <TextDistributionTool devices={devices} targets={targets} targetDevices={targetDevices} />
          )}
          {tool === "files" && (
            <FileDistributionTool devices={devices} targets={targets} targetDevices={targetDevices} />
          )}
          {tool === "reply" && <QuickReplyTool targets={targets} scopeLabel={scopeLabel} />}
          {tool === "keys" && <QuickActionsTool targets={targets} scopeLabel={scopeLabel} />}
          {tool === "macro" && <MacroTool targets={targets} scopeLabel={scopeLabel} />}
          {tool === "gps" && <GpsTool targets={targets} scopeLabel={scopeLabel} />}
          {tool === "root" && <RootTool targets={targets} scopeLabel={scopeLabel} />}
          {tool === "peripherals" && <PeripheralsTool targets={targets} scopeLabel={scopeLabel} />}
        </div>
      </div>
    </div>
  );
}
