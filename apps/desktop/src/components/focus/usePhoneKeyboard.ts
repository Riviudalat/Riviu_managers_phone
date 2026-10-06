import { useLayoutEffect, useRef, type ClipboardEvent, type KeyboardEvent, type FormEvent } from "react";
import { deviceKeyboardInput, devicePasteText, type PhoneKeyboardInput } from "../../api";
import { pushToast, toastError } from "../../toastStore";

type Action = PhoneKeyboardInput | { kind: "paste"; text: string };
const editingKeys = new Set(["Backspace", "Delete", "Enter", "Tab", "ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End", "PageUp", "PageDown"]);

/** The textarea owns text/IME; keydown owns editing keys only. All effects share one FIFO. */
export function usePhoneKeyboard(options: {
  udid: string; generation?: number; enabled: boolean; blocked: boolean;
  unsupported?: "group" | "ios"; sessionKey: string;
  isExternallyBusy: () => boolean;
}) {
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const current = useRef(options);
  useLayoutEffect(() => { current.current = options; });
  const state = useRef({ epoch: 0, queue: [] as Action[], running: false, stopped: false, composing: false, committed: "", warned: false });
  const invalidate = () => {
    const s = state.current;
    s.epoch++; s.queue = []; s.composing = false; s.committed = "";
    if (inputRef.current) inputRef.current.value = "";
  };
  useLayoutEffect(() => {
    invalidate();
    state.current.stopped = false;
    state.current.warned = false;
    return invalidate;
  }, [options.udid, options.generation, options.enabled, options.blocked, options.unsupported, options.sessionKey]);
  useLayoutEffect(() => {
    window.addEventListener("blur", invalidate);
    return () => window.removeEventListener("blur", invalidate);
  }, []);
  const warn = (message: string) => {
    if (!state.current.warned) pushToast("warn", message);
    state.current.warned = true;
  };
  const drain = async () => {
    const s = state.current;
    if (s.running) return;
    s.running = true;
    try {
      while (s.queue.length && !s.stopped) {
        const o = current.current;
        if (!o.enabled || o.blocked || o.isExternallyBusy() || o.unsupported || o.generation === undefined) { invalidate(); break; }
        const action = s.queue.shift()!;
        const identity = s.epoch;
        const { udid, generation } = current.current;
        try {
          if (action.kind === "paste") await devicePasteText(udid, action.text, generation!);
          else {
            const result = await deviceKeyboardInput(udid, generation!, action);
            if (action.kind === "copy" && identity === s.epoch) {
              if (typeof result.text !== "string") throw new Error("Điện thoại chưa trả về văn bản mới để sao chép");
              await navigator.clipboard.writeText(result.text);
            }
          }
        } catch (error) {
          if (identity === s.epoch) {
            s.queue = []; s.stopped = true; s.warned = true;
            toastError("Chưa xác nhận bàn phím; kiểm tra điện thoại rồi bấm lại màn hình để tiếp tục", error);
          }
        }
      }
    } finally { s.running = false; }
  };
  const enqueue = (action: Action) => {
    const o = current.current;
    if (o.unsupported) {
      warn(o.unsupported === "group" ? "Chưa hỗ trợ bàn phím PC cho nhóm máy" : "Bàn phím PC chỉ hỗ trợ Android");
      return;
    }
    if (!o.enabled || o.blocked || o.isExternallyBusy() || o.generation === undefined) { warn("Chưa sẵn sàng nhận bàn phím điện thoại"); return; }
    const s = state.current;
    if (s.stopped) return;
    if (s.queue.length >= 256 || ("text" in action && action.text.length > 65536)) {
      s.queue = []; s.stopped = true;
      warn("Hàng đợi bàn phím quá đầy; đã hủy phần chờ. Kiểm tra điện thoại rồi bấm lại màn hình.");
      return;
    }
    s.queue.push(action);
    void drain();
  };
  const owns = (target: EventTarget) => target === inputRef.current || target === inputRef.current?.parentElement;
  const onKeyDown = (event: KeyboardEvent) => {
    if (!owns(event.target) || !owns(document.activeElement!)) return;
    const s = state.current;
    if (s.composing || event.nativeEvent.isComposing || event.keyCode === 229) return;
    s.committed = "";
    const ctrl = event.ctrlKey || event.metaKey;
    const key = ctrl && event.key.length === 1 ? event.key.toLowerCase() : event.key;
    if (ctrl && key === "v") return; // Native paste is the sole clipboard input owner.
    if (ctrl && (key === "c" || key === "x")) {
      event.preventDefault(); event.stopPropagation();
      if (!event.repeat) enqueue({ kind: "copy", cut: key === "x" });
      return;
    }
    if (editingKeys.has(event.key) || (ctrl && ["a", "z", "y"].includes(key))) {
      event.preventDefault(); event.stopPropagation();
      enqueue({ kind: "key", key, shift: event.shiftKey, ctrl, alt: event.altKey, repeat: event.repeat });
    }
  };
  const onPaste = (event: ClipboardEvent) => {
    if (!owns(event.target) || !owns(document.activeElement!)) return;
    event.preventDefault(); event.stopPropagation();
    const text = event.clipboardData.getData("text/plain");
    if (text) enqueue({ kind: "paste", text });
  };
  const onInput = (event: FormEvent<HTMLTextAreaElement>) => {
    const s = state.current;
    if (s.composing) return;
    if (document.activeElement !== event.currentTarget) { event.currentTarget.value = ""; return; }
    const text = event.currentTarget.value;
    event.currentTarget.value = "";
    if (s.committed && (text === s.committed || (event.nativeEvent as InputEvent).data === s.committed)) { s.committed = ""; return; }
    s.committed = "";
    if (text) enqueue({ kind: "text", text });
  };
  return {
    inputRef,
    isBusy: () => state.current.running || state.current.queue.length > 0,
    focus: () => {
      if (state.current.stopped) { invalidate(); state.current.stopped = false; }
      state.current.warned = false;
      inputRef.current?.focus({ preventScroll: true });
    },
    surfaceProps: {
      onKeyDown, onPaste,
      onFocus: (event: React.FocusEvent<HTMLDivElement>) => {
        if (event.target === event.currentTarget) inputRef.current?.focus({ preventScroll: true });
      },
      onBlur: (event: React.FocusEvent<HTMLDivElement>) => {
        if (!event.currentTarget.contains(event.relatedTarget as Node | null)) invalidate();
      },
    },
    inputProps: {
      onInput,
      onCompositionStart: () => { state.current.composing = true; state.current.committed = ""; },
      onCompositionEnd: (event: React.CompositionEvent<HTMLTextAreaElement>) => {
        if (!state.current.composing) return;
        state.current.composing = false;
        const text = event.data || event.currentTarget.value;
        state.current.committed = text;
        event.currentTarget.value = "";
        if (text) enqueue({ kind: "text", text });
      },
    },
  };
}
