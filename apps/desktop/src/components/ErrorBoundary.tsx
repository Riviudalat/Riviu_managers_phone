import { Component, type ErrorInfo, type ReactNode } from "react";
import { describeError } from "../describeError";

type Props = {
  children: ReactNode;
  /** Where the failure goes. Injected so a test can assert without a Tauri bridge. */
  onError?: (message: string, source?: string) => void;
};

type State = { message: string | null };

/**
 * **Catch a render throw, because otherwise React unmounts everything and leaves a blank
 * window.**
 *
 * With no boundary anywhere in the tree, one component throwing during render takes the whole
 * app down to an empty `<div id="root">`. The operator sees a white screen — no message, no
 * toast, and until `log_frontend_error` existed, no log line either. That is the shape of the
 * report this came from: *"lên app rồi… không có gì xảy ra."*
 *
 * This is also the production form of a trap that has already reddened this suite three times:
 * an export missing from an object-literal `vi.mock` is `undefined`, and calling it throws
 * synchronously mid-render. In tests that surfaced as six failures at once; in a shipped build
 * it surfaced as nothing at all.
 *
 * Kept deliberately plain: a message, and a way out. It does **not** offer "try again" —
 * re-rendering the subtree that just threw usually throws again, and a button that appears to
 * do nothing is worse than no button. Reload is the honest action.
 */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { message: null };

  static getDerivedStateFromError(error: unknown): State {
    return { message: describeError(error) };
  }

  componentDidCatch(error: unknown, info: ErrorInfo) {
    // `componentStack` is the part that says *which* component, which the message never does.
    const source = info.componentStack?.trim().split("\n")[0]?.trim();
    this.props.onError?.(describeError(error), source || undefined);
  }

  render() {
    if (this.state.message === null) return this.props.children;
    return (
      <div role="alert" className="crash-shell">
        <h1>Giao diện gặp lỗi và đã dừng</h1>
        <p className="crash-message">{this.state.message}</p>
        {/*
          Says what is known and no more. The backend process keeps running, so a reload
          rebuilds the interface rather than restarting the app -- but whether an overlay
          control session survives a reload is not established here, so it is not promised.
        */}
        <p className="crash-hint">
          Lỗi này đã được ghi vào log của app. Tải lại giao diện để tiếp tục; phần điều khiển
          máy chạy ở tiến trình riêng nên nó không bị tắt theo.
        </p>
        <button type="button" onClick={() => window.location.reload()}>
          Tải lại giao diện
        </button>
      </div>
    );
  }
}
