import { useCallback, useEffect, useState } from "react";

import { syslog } from "../api";
import { describeError } from "../describeError";
import type { DeviceInfo } from "../types";

/** How many lines to ask the phone for. Enough to cover a crash and its lead-up. */
const LINES = 400;

/**
 * The phone's own log, on demand.
 *
 * **`syslog` was a registered command with no caller.** The command, the `Driver::syslog_tail`
 * trait method and its seven test mocks all existed; `api.ts` never invoked it, so the only way
 * to read a phone's log from this app was not to. That matters most in exactly the situation
 * this whole pass came from — a phone that lists and will not drive — because the phone's own
 * log is where the reason is.
 *
 * **It parks the live producer while it runs**, because `syslog` takes the lease with
 * `LeaseStream::Park`. So the tile goes quiet for the duration and comes back after. Said out
 * loud in the panel rather than left for the operator to notice: a tile going blank the moment
 * you ask for a log looks like the thing you were investigating.
 */
export function DeviceSyslogPopup({
  device,
  onClose,
}: {
  device: DeviceInfo;
  onClose: () => void;
}) {
  const [text, setText] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(() => {
    setBusy(true);
    setError(null);
    void syslog(device.udid, LINES)
      .then((next) => setText(next))
      .catch((cause) => setError(describeError(cause)))
      .finally(() => setBusy(false));
  }, [device.udid]);

  useEffect(load, [load]);

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal device-syslog"
        role="dialog"
        aria-modal="true"
        aria-label={`Log của ${device.name}`}
        onClick={(event) => event.stopPropagation()}
      >
        <header className="panel-header">
          <h2>Log của {device.name}</h2>
          <span className="grow" />
          <button type="button" className="ghost" onClick={load} disabled={busy}>
            {busy ? "Đang đọc…" : "Đọc lại"}
          </button>
          <button type="button" className="ghost" onClick={onClose}>
            Đóng
          </button>
        </header>

        <p className="hint">
          {LINES} dòng cuối, đọc trực tiếp từ máy. Trong lúc đọc, luồng hình của máy này tạm dừng
          rồi tự chạy lại — tile tối đi là chuyện bình thường, không phải lỗi.
        </p>

        {error && (
          <p className="hint" role="alert">
            {error}
          </p>
        )}
        {!error && text === null && <p className="hint">Đang đọc log từ máy…</p>}
        {!error && text !== null && text.trim() === "" && (
          <p className="hint">Máy trả về log rỗng.</p>
        )}
        {!error && text !== null && text.trim() !== "" && (
          <pre className="device-syslog-body">{text}</pre>
        )}
      </div>
    </div>
  );
}
