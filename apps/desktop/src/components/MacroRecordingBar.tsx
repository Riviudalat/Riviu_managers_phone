import { Circle, Square } from "lucide-react";
import { useRecordedSteps } from "../macroStore";

/** The same recording follows the operator between the shell and the phone menu. */
export function MacroRecordingBar({ onStop }: { onStop: () => void }) {
  const steps = useRecordedSteps();
  return (
    <section className="macro-recording-bar" aria-label="Ghi Macro">
      <div className="macro-recording-summary">
        <Circle size={10} fill="currentColor" aria-hidden="true" />
        <strong>Đang ghi Macro</strong>
        <span role="status" aria-live="polite">{steps.length} bước</span>
      </div>
      <button type="button" onClick={onStop}>
        <Square size={14} aria-hidden="true" />Dừng ghi
      </button>
    </section>
  );
}
