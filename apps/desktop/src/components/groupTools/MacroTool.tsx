import { useRef, useState } from "react";
import { groupInput } from "../../api";
import { getGroupSync } from "../../groupSync";
import { groupInputOutcome } from "../../groupInput";
import type { GroupInputReport } from "../../types";
import { expand, stepSummary, totalWaitMs, type Macro } from "../../macro";
import {
  clearRecording,
  deleteMacro,
  recordedSteps,
  saveMacro,
  startRecording,
  stopRecording,
  useMacroRecording,
  useRecordedSteps,
  useSavedMacros,
} from "../../macroStore";
import { pushToast } from "../../toastStore";
import { sleep } from "./toolHelpers";

export function MacroTool({ targets, scopeLabel }: { targets: string[]; scopeLabel: string }) {
  const recording = useMacroRecording();
  const steps = useRecordedSteps();
  const macros = useSavedMacros();
  const [name, setName] = useState("");
  const [loops, setLoops] = useState(1);
  const [playing, setPlaying] = useState<string | null>(null);
  const stopRef = useRef(false);

  const save = () => {
    const macro = saveMacro(name, recordedSteps());
    if (macro) {
      pushToast("ok", "Đã lưu macro", `${macro.name} · ${macro.steps.length} bước`);
      setName("");
    } else {
      pushToast("warn", "Chưa có bước nào", "Bật ghi rồi thao tác trên overlay điều khiển.");
    }
  };

  const play = async (macro: Macro) => {
    if (!targets.length) {
      pushToast("warn", "Chưa có máy", "Chọn máy rồi chạy.");
      return;
    }
    const plan = expand(macro.steps, loops);
    setPlaying(macro.id);
    stopRef.current = false;
    let failed = 0;
    // **A step that reached no phone is a failed step.** `failed` only ever counted a
    // rejected invocation, and `groupInput` does not reject when every phone is skipped --
    // it resolves with them all in `report.skipped`. So running a macro against twenty
    // phones that were all busy with nurture reported "Đã chạy macro" having performed
    // nothing at all. `QuickActionsTool` and `QuickReplyTool` already read the report
    // through `groupInputOutcome`; this tool and `PeripheralsTool` were the two that did
    // not. Found by an independent review on 27/08/2026.
    let unreached = 0;
    let lastOutcome: string | null = null;
    const account = (report: GroupInputReport) => {
      const outcome = groupInputOutcome(report);
      if (outcome.kind === "ok") return;
      unreached += 1;
      lastOutcome = outcome.detail;
    };
    try {
      for (const step of plan) {
        if (stopRef.current) break;
        try {
          if (step.kind === "tap") {
            account(await groupInput({
              udids: targets,
              kind: "tap",
              x: step.x,
              y: step.y,
              imageW: step.iw,
              imageH: step.ih,
              sync: getGroupSync(),
            }));
          } else if (step.kind === "swipe") {
            account(await groupInput({
              udids: targets,
              kind: "swipe",
              x: step.x,
              y: step.y,
              toX: step.toX,
              toY: step.toY,
              imageW: step.iw,
              imageH: step.ih,
              sync: getGroupSync(),
            }));
          } else if (step.kind === "key") {
            account(
              await groupInput({
                udids: targets,
                kind: "key",
                key: step.key,
                sync: getGroupSync(),
              }),
            );
          }
        } catch {
          failed += 1;
        }
        if (step.afterMs > 0 && !stopRef.current) await sleep(step.afterMs);
      }
      if (stopRef.current) pushToast("warn", "Đã dừng macro", macro.name);
      else if (failed || unreached)
        pushToast(
          "warn",
          "Macro chạy xong (có lỗi)",
          [
            failed ? `${failed} bước lỗi` : null,
            unreached ? `${unreached} bước không tới được máy nào hoặc bị bỏ qua` : null,
            lastOutcome,
          ]
            .filter(Boolean)
            .join("\n"),
        );
      else pushToast("ok", "Đã chạy macro", `${macro.name} × ${loops}`);
    } finally {
      setPlaying(null);
    }
  };

  return (
    <>
      <p className="hint">
        Ghi thao tác trên một máy rồi phát lại cho {scopeLabel}. Bật ghi, mở "Mở điều khiển"
        một máy rồi chạm/vuốt/bấm phím — mỗi bước được ghi theo toạ độ ảnh và phát lại đúng vị
        trí trên từng máy (kèm delay/offset nếu bật).
      </p>
      <div className="nurture-float-actions">
        {recording ? (
          <button type="button" className="primary" onClick={() => stopRecording()}>
            Dừng ghi ({steps.length})
          </button>
        ) : (
          <button type="button" className="primary" onClick={() => startRecording()}>
            Bắt đầu ghi
          </button>
        )}
        <button
          type="button"
          className="ghost"
          disabled={!steps.length}
          onClick={() => clearRecording()}
        >
          Xoá bản ghi
        </button>
      </div>
      {steps.length > 0 && (
        <>
          <div className="group-tools-preview" style={{ maxHeight: 140 }}>
            {steps.map((s, i) => (
              <div className="row-item" key={i}>
                <span className="who">#{i + 1}</span>
                <span className="what">{stepSummary(s)}</span>
              </div>
            ))}
          </div>
          <div className="row" style={{ marginTop: "0.4rem" }}>
            <input
              type="text"
              placeholder="Tên macro"
              value={name}
              onChange={(e) => setName(e.target.value)}
            />
            <button type="button" className="ghost" disabled={recording} onClick={save}>
              Lưu macro
            </button>
          </div>
        </>
      )}

      <div className="row" style={{ marginTop: "0.6rem" }}>
        <label>
          Số vòng lặp
          <input
            type="number"
            min={1}
            value={loops}
            onChange={(e) => setLoops(Math.max(1, Math.round(Number(e.target.value) || 1)))}
          />
        </label>
      </div>
      <p className="hint">Macro đã lưu ({macros.length})</p>
      <div className="group-tools-preview">
        {macros.length === 0 ? (
          <div className="row-item">
            <span className="hint">Chưa có macro nào.</span>
          </div>
        ) : (
          macros.map((m) => (
            <div className="row-item" key={m.id}>
              <span className="who" title={m.name}>
                {m.name}
              </span>
              <span className="what">
                {m.steps.length} bước · ~{Math.round(totalWaitMs(m.steps, loops) / 100) / 10}s chờ
              </span>
              <span className="grow" />
              {playing === m.id ? (
                <button
                  type="button"
                  className="ghost"
                  onClick={() => {
                    stopRef.current = true;
                  }}
                >
                  Dừng
                </button>
              ) : (
                <button
                  type="button"
                  className="ghost"
                  disabled={playing !== null || recording}
                  onClick={() => void play(m)}
                >
                  Chạy
                </button>
              )}
              <button type="button" className="ghost" onClick={() => deleteMacro(m.id)}>
                Xoá
              </button>
            </div>
          ))
        )}
      </div>
    </>
  );
}
