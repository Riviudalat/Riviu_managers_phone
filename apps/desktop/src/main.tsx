import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "@xyflow/react/dist/style.css";
import "./index.css";
import App from "./App.tsx";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { bootMarkerVerdict, installCrashReporting } from "./crashReport";
import { logFrontendError } from "./api";

const rootElement = document.getElementById("root")!;

/**
 * Everything uncaught goes to the app log from here on.
 *
 * `index.html` installs its own `error`/`unhandledrejection` pair, but those only write into
 * `#boot-marker` — and this file removes that element the moment React mounts, so they fall
 * silent for the entire running life of the app. These handlers replace that silence: they use
 * `describeError` (so a `{ code, message }` rejection is not `[object Object]`) and they reach
 * `log_frontend_error`, which is rate-limited on the Rust side so a render loop cannot flood
 * the file.
 */
installCrashReporting((kind, message, source) => {
  void logFrontendError(kind, message, source);
});

createRoot(rootElement).render(
  <StrictMode>
    <ErrorBoundary
      onError={(message, source) => {
        void logFrontendError("render", message, source);
      }}
    >
      <App />
    </ErrorBoundary>
  </StrictMode>,
);

const startedAt = performance.now();
const settleBootMarker = () => {
  const marker = document.getElementById("boot-marker");
  if (!marker) return;
  switch (bootMarkerVerdict(rootElement.childElementCount > 0, performance.now() - startedAt)) {
    case "clear":
      marker.remove();
      return;
    case "stuck":
      // Say it rather than spinning. The reason is already in the app log by now: whatever
      // stopped the mount went through `installCrashReporting` above.
      marker.textContent =
        "Giao diện không khởi động được. Lý do đã ghi trong log của app (Cài đặt → mở thư mục log).";
      return;
    default:
      window.requestAnimationFrame(settleBootMarker);
  }
};
window.requestAnimationFrame(settleBootMarker);
