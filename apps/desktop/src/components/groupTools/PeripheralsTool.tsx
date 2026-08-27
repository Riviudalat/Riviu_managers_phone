import { useEffect, useRef, useState } from "react";

/// How long between two complaints about the same fleet refusing input.
///
/// A gamepad polls per frame, so an unthrottled warning would push a toast sixty times a
/// second at a fleet that is entirely held by another owner. Three seconds is long enough
/// that the operator reads one message, short enough that it reappears if they keep pressing.
const COMPLAIN_THROTTLE_MS = 3000;
import {
  groupInput,
  listSerialPorts,
  relayPulseChannel,
  relaySetChannel,
  type SerialPortInfo,
} from "../../api";
import {
  defaultGamepadBindings,
  REFERENCE,
  resolveButtonAction,
  risingEdges,
  toReference,
  type PeripheralAction,
} from "../../peripheralMap";
import { getGroupSync } from "../../groupSync";
import { groupInputOutcome } from "../../groupInput";
import { describeError } from "../../describeError";
import type { GroupInputReport } from "../../types";
import { pushToast, toastError } from "../../toastStore";

/** Human label for a peripheral action, shown as "vừa gửi …" feedback. */
function describeAction(action: PeripheralAction): string {
  switch (action.kind) {
    case "key":
      return `phím ${action.key}`;
    case "tap":
      return "chạm";
    case "swipe":
      return "vuốt";
    case "macro":
      return `macro ${action.name}`;
  }
}

/**
 * Physical peripherals (D, xiaowei "外设"): USB relay power control + a gamepad→fleet bridge.
 *
 * The relay talks to the host serial port through the backend; the gamepad is read here with
 * the browser's Web Gamepad API (no host driver) and mapped to fleet gestures by
 * `peripheralMap`. Both act on the current selection.
 */
export function PeripheralsTool({ targets, scopeLabel }: { targets: string[]; scopeLabel: string }) {
  const [ports, setPorts] = useState<SerialPortInfo[]>([]);
  const [port, setPort] = useState("");
  const [channel, setChannel] = useState(1);
  const [holdMs, setHoldMs] = useState(800);
  const [energize, setEnergize] = useState(true);
  const [busy, setBusy] = useState<string | null>(null);
  const [padOn, setPadOn] = useState(false);
  const [padId, setPadId] = useState<string | null>(null);
  const [lastFired, setLastFired] = useState<string | null>(null);
  /// The last time a gamepad press reached no phone, or the call itself failed.
  ///
  /// **"Sent" used to be claimed unconditionally.** `fire` dispatched with `void groupInput(..)`
  /// and immediately set `lastFired`, rendered as "vừa gửi". `groupInput` does not reject when
  /// every phone is skipped -- it resolves with them all in `report.skipped` -- so pressing Home
  /// with the whole fleet held by another owner showed "vừa gửi phím home" having sent nothing.
  /// An IPC rejection was equally invisible: nothing local handled it.
  ///
  /// Found by an independent review on 27/08/2026.
  const [lastProblem, setLastProblem] = useState<string | null>(null);

  // The poll loop reads the selection through a ref so it need not restart on every
  // selection change (which would reset edge-detection mid-press).
  const targetsRef = useRef(targets);
  targetsRef.current = targets;

  const refreshPorts = async () => {
    setBusy("ports");
    try {
      const found = await listSerialPorts();
      setPorts(found);
      if (!port && found.length) setPort(found[0].name);
    } catch (e) {
      toastError("Không liệt kê được cổng", e);
    } finally {
      setBusy(null);
    }
  };

  const relaySet = async (on: boolean) => {
    if (!port) {
      pushToast("warn", "Chưa chọn cổng", "Chọn cổng COM của bo relay.");
      return;
    }
    setBusy(on ? "on" : "off");
    try {
      await relaySetChannel(port, channel, on);
      pushToast("ok", on ? "Đã bật kênh relay" : "Đã tắt kênh relay", `${port} · kênh ${channel}`);
    } catch (e) {
      toastError("Lệnh relay thất bại", e);
    } finally {
      setBusy(null);
    }
  };

  const relayPulse = async () => {
    if (!port) {
      pushToast("warn", "Chưa chọn cổng", "Chọn cổng COM của bo relay.");
      return;
    }
    setBusy("pulse");
    try {
      await relayPulseChannel(port, channel, holdMs, energize);
      pushToast("ok", "Đã xung relay", `${port} · kênh ${channel} · ${holdMs}ms`);
    } catch (e) {
      toastError("Xung relay thất bại", e);
    } finally {
      setBusy(null);
    }
  };

  // Gamepad → fleet bridge. Polls on animation frames while enabled; fires each bound button
  // once on its rising edge (see `risingEdges`) to the current selection.
  useEffect(() => {
    if (!padOn) return;
    const bindings = defaultGamepadBindings();
    let previous: boolean[] = [];
    let frame = 0;

    // A gamepad fires at frame rate, so the dispatch stays non-blocking -- awaiting each
    // press would put fleet latency into the stick. What changes is that the *outcome* is
    // now read, and a bad one is throttled rather than dropped: without a throttle a fleet
    // that rejects everything would push a toast every frame.
    let lastComplaint = 0;
    const complain = (title: string, detail: string) => {
      setLastProblem(title);
      const now = performance.now();
      if (now - lastComplaint < COMPLAIN_THROTTLE_MS) return;
      lastComplaint = now;
      pushToast("warn", title, detail);
    };
    const watch = (sent: Promise<GroupInputReport>, what: string) => {
      sent
        .then((report) => {
          const outcome = groupInputOutcome(report);
          if (outcome.kind !== "ok") complain(`${what}: ${outcome.title}`, outcome.detail);
        })
        .catch((error) => complain(`${what} thất bại`, describeError(error)));
    };

    const fire = (action: PeripheralAction) => {
      const udids = targetsRef.current;
      if (!udids.length) return;
      const sync = getGroupSync();
      if (action.kind === "key") {
        watch(groupInput({ udids, kind: "key", key: action.key, sync }), describeAction(action));
      } else if (action.kind === "tap") {
        watch(
          groupInput({
          udids,
          kind: "tap",
            x: toReference(action.fx),
            y: toReference(action.fy),
            imageW: REFERENCE,
            imageH: REFERENCE,
            sync,
          }),
          describeAction(action),
        );
      } else if (action.kind === "swipe") {
        watch(
          groupInput({
          udids,
          kind: "swipe",
            x: toReference(action.fx1),
            y: toReference(action.fy1),
            toX: toReference(action.fx2),
            toY: toReference(action.fy2),
            imageW: REFERENCE,
            imageH: REFERENCE,
            sync,
          }),
          describeAction(action),
        );
      }
      // macro bindings are not in the default set; nothing to fire here.
      //
      // This records that the press was **dispatched**, which is all that is known
      // synchronously. Whether a phone took it arrives later, in `watch`.
      setLastFired(describeAction(action));
      setLastProblem(null);
    };

    const poll = () => {
      const pads = navigator.getGamepads ? navigator.getGamepads() : [];
      const pad = Array.from(pads).find((p): p is Gamepad => p !== null);
      if (pad) {
        setPadId(pad.id);
        const current = pad.buttons.map((b) => b.pressed);
        for (const index of risingEdges(previous, current)) {
          const action = resolveButtonAction(bindings, index);
          if (action) fire(action);
        }
        previous = current;
      } else {
        setPadId(null);
      }
      frame = requestAnimationFrame(poll);
    };
    frame = requestAnimationFrame(poll);
    return () => cancelAnimationFrame(frame);
  }, [padOn]);

  return (
    <>
      <p className="hint">
        Ngoại vi vật lý cho {scopeLabel} (xiaowei "外设"). Relay USB để bật/tắt nguồn hoặc khởi
        động cứng máy kẹt; tay cầm (Gamepad) điều khiển cả nhóm.
      </p>

      <fieldset className="group-tools-fieldset">
        <legend>USB Relay (nguồn / reboot cứng)</legend>
        <div className="row">
          <label style={{ flex: 1 }}>
            Cổng COM
            <select value={port} onChange={(e) => setPort(e.target.value)}>
              <option value="">— chọn cổng —</option>
              {ports.map((p) => (
                <option key={p.name} value={p.name}>
                  {p.name} ({p.kind})
                </option>
              ))}
            </select>
          </label>
          <label>
            Kênh
            <input
              type="number"
              min={1}
              max={16}
              value={channel}
              onChange={(e) => setChannel(Math.max(1, Number(e.target.value) || 1))}
              style={{ width: "5rem" }}
            />
          </label>
          <button type="button" className="ghost" disabled={busy !== null} onClick={() => void refreshPorts()}>
            {busy === "ports" ? "…" : "Quét cổng"}
          </button>
        </div>
        <div className="nurture-float-actions">
          <button type="button" className="ghost" disabled={busy !== null} onClick={() => void relaySet(true)}>
            {busy === "on" ? "…" : "Bật kênh"}
          </button>
          <button type="button" className="ghost" disabled={busy !== null} onClick={() => void relaySet(false)}>
            {busy === "off" ? "…" : "Tắt kênh"}
          </button>
        </div>
        <div className="row" style={{ marginTop: "0.4rem" }}>
          <label>
            Giữ (ms)
            <input
              type="number"
              min={50}
              max={10000}
              value={holdMs}
              onChange={(e) => setHoldMs(Math.max(50, Number(e.target.value) || 50))}
              style={{ width: "6rem" }}
            />
          </label>
          <label className="check">
            <input type="checkbox" checked={energize} onChange={(e) => setEnergize(e.target.checked)} />
            Nhấn (bật→tắt); bỏ tích = ngắt nguồn (tắt→bật)
          </label>
          <button type="button" className="primary" disabled={busy !== null} onClick={() => void relayPulse()}>
            {busy === "pulse" ? "…" : "Xung (reboot)"}
          </button>
        </div>
      </fieldset>

      <fieldset className="group-tools-fieldset">
        <legend>Tay cầm (Gamepad) → nhóm</legend>
        <p className="hint">
          Cắm tay cầm USB/Bluetooth vào PC. A→Home, B→Back, X→Đa nhiệm, D-pad→vuốt. Mỗi lần bấm
          gửi cho {scopeLabel}.
        </p>
        <label className="check">
          <input type="checkbox" checked={padOn} onChange={(e) => setPadOn(e.target.checked)} />
          Bật điều khiển bằng tay cầm
        </label>
        {padOn && (
          <p className="hint">
            {padId ? `Đã nhận: ${padId}` : "Chưa thấy tay cầm — bấm một nút để trình duyệt nhận."}
            {lastFired ? ` · vừa gửi ${lastFired}` : ""}
            {lastProblem ? ` · ⚠ ${lastProblem}` : ""}
          </p>
        )}
      </fieldset>
    </>
  );
}
