import { useRef, useState } from "react";
import { Send } from "lucide-react";
import { deviceTypeText, groupInput } from "../../api";
import { describeError } from "../../describeError";
import { getGroupSync } from "../../groupSync";
import { groupInputOutcome } from "../../groupInput";
import type { GroupInputReport } from "../../types";
import { MAX_QUICK_PHRASE_LENGTH } from "../../quickPhrases";
import "./focus-text-input.css";

export type FocusTextInputProps = {
  udid: string;
  targets: string[];
  masterUdid?: string;
  ready: boolean;
  busy: boolean;
  runBusy: (work: () => Promise<void>) => Promise<boolean>;
  reportGroup: (report: GroupInputReport, quiet: boolean) => boolean;
};

export function FocusTextInput({ udid, targets, masterUdid, ready, busy, runBusy, reportGroup }: FocusTextInputProps) {
  const [text, setText] = useState("");
  const [pending, setPending] = useState(false);
  const [feedback, setFeedback] = useState<{ text: string; error: boolean } | null>(null);
  const sending = useRef(false);
  const canSend = ready && !busy && !pending && text.trim().length > 0 && targets.length > 0;

  const send = async () => {
    if (!canSend || sending.current) return;
    const submittedText = text;
    sending.current = true;
    setPending(true);
    setFeedback(null);
    try {
      let delivered = false;
      let message = "";
      let error = false;
      const ran = await runBusy(async () => {
        if (targets.length === 1) {
          await deviceTypeText(udid, submittedText);
          delivered = true;
          message = "Đã gửi chữ vào máy.";
          return;
        }
        const report = await groupInput({ udids: targets, masterUdid, kind: "type", text: submittedText, sync: getGroupSync() });
        reportGroup(report, false);
        const outcome = groupInputOutcome(report);
        delivered = report.completedUdids.length > 0;
        error = outcome.kind !== "ok";
        message = outcome.kind === "none"
          ? "Không máy nào nhận được chữ."
          : `Đã gửi ${report.completedUdids.length}/${targets.length} máy${error ? "; kiểm tra máy bị bỏ qua." : "."}`;
      });
      if (ran) {
        if (delivered) setText(current => current === submittedText ? "" : current);
        setFeedback({ text: message, error });
      } else {
        setFeedback({ text: "Máy đang xử lý thao tác khác.", error: true });
      }
    } catch (cause) {
      setFeedback({ text: `Không gửi được chữ: ${describeError(cause)}`, error: true });
    } finally {
      sending.current = false;
      setPending(false);
    }
  };

  return <div className="focus-text-entry" role="group" aria-label="Nhập chữ vào máy">
    <div className="focus-text-entry-row">
      <textarea
        aria-label="Nhập chữ vào máy"
        rows={2}
        maxLength={MAX_QUICK_PHRASE_LENGTH}
        value={text}
        disabled={busy}
        placeholder="Nhập chữ..."
        onChange={event => { setText(event.target.value); setFeedback(null); }}
        onKeyDown={event => {
          event.stopPropagation();
          if (event.key !== "Enter" || event.shiftKey || event.nativeEvent.isComposing || event.keyCode === 229) return;
          event.preventDefault();
          void send();
        }}
        onKeyUp={event => event.stopPropagation()}
      />
      <button type="button" aria-label="Gửi chữ" title="Gửi chữ vào ô đang chọn trên máy" disabled={!canSend} onClick={() => void send()}>
        <Send size={16} aria-hidden="true" />
      </button>
    </div>
    {feedback && <p role={feedback.error ? "alert" : "status"} className={feedback.error ? "is-error" : undefined}>{feedback.text}</p>}
  </div>;
}
