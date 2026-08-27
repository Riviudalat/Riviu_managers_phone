import { useState } from "react";
import { describeError } from "../describeError";
import { deviceShell } from "../api";
import type { DeviceInfo } from "../types";

interface Props {
  device: DeviceInfo;
  onClose: () => void;
}

/**
 * A one-line shell for one phone.
 *
 * `adb shell <script>` and nothing else — the backend exposes no path to
 * `adb <subcommand>`, so `install`, `reboot`, `root` and `kill-server` are not one typo
 * away. The last of those matters most: it would tear down every other tool's adb
 * connection on this machine.
 *
 * The command takes an exclusive lease on the device, so a phone another piece of work
 * is holding refuses rather than races. That refusal arrives as the error text and is
 * shown as-is; it names the current owner, which is the thing the operator needs.
 */
export function AdbConsole({ device, onClose }: Props) {
  const [script, setScript] = useState("");
  const [output, setOutput] = useState<{ text: string; failed: boolean } | null>(null);
  const [running, setRunning] = useState(false);

  const run = async () => {
    if (!script.trim() || running) return;
    setRunning(true);
    // Cleared so a slow command cannot leave the previous command's output on screen
    // looking like this one's answer.
    setOutput(null);
    try {
      const result = await deviceShell(device.udid, script);
      // stderr and the exit code are shown alongside stdout rather than swallowed: a
      // non-zero exit is a normal answer here, and the message is usually on stderr.
      const parts = [result.stdout.trimEnd(), result.stderr.trimEnd()].filter(Boolean);
      const body = parts.length ? parts.join("\n") : "(không có output)";
      setOutput({
        text: result.exitCode === 0 ? body : `exit ${result.exitCode}\n${body}`,
        failed: result.exitCode !== 0,
      });
    } catch (error) {
      // Reached only for a real failure: adb gone, device unplugged, lease refused.
      // `describeError`: the refusal arrives as `{code, message}` and `String` of that is
      // "[object Object]" — exactly the sentence an operator cannot act on.
      setOutput({ text: describeError(error), failed: true });
    } finally {
      setRunning(false);
    }
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal adb-console"
        role="dialog"
        aria-modal="true"
        aria-label={`Lệnh adb trên ${device.name}`}
        onClick={(event) => event.stopPropagation()}
      >
        <header>
          <strong>adb shell — {device.name}</strong>
          <button type="button" className="ghost" onClick={onClose}>
            Đóng
          </button>
        </header>
        <p className="hint">
          Chỉ <code>adb shell</code>. Không có đường tới <code>adb install</code>,{" "}
          <code>reboot</code> hay <code>kill-server</code> từ đây.
        </p>
        <div className="row">
          <input
            value={script}
            autoFocus
            placeholder="ví dụ: getprop ro.build.version.release"
            aria-label="Lệnh shell"
            onChange={(event) => setScript(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") void run();
            }}
          />
          <button type="button" className="primary" disabled={running} onClick={() => void run()}>
            {running ? "Đang chạy..." : "Chạy"}
          </button>
        </div>
        {output !== null && (
          <pre className={`adb-output ${output.failed ? "failed" : ""}`}>{output.text}</pre>
        )}
      </div>
    </div>
  );
}
