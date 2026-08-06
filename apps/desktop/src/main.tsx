import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "@xyflow/react/dist/style.css";
import "./index.css";
import App from "./App.tsx";

const rootElement = document.getElementById("root")!;
createRoot(rootElement).render(
  <StrictMode>
    <App />
  </StrictMode>,
);

const removeBootMarkerWhenMounted = () => {
  if (rootElement.childElementCount > 0) {
    document.getElementById("boot-marker")?.remove();
    return;
  }
  window.requestAnimationFrame(removeBootMarkerWhenMounted);
};
window.requestAnimationFrame(removeBootMarkerWhenMounted);
