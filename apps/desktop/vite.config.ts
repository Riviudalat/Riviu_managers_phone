import react from "@vitejs/plugin-react";
import { configDefaults, defineConfig } from "vitest/config";

// https://vite.dev/config/
export default defineConfig({
  // Tauri serves production assets from its local protocol origin. Relative
  // URLs keep the embedded HTML, JS and CSS on that origin on every platform.
  base: "./",
  plugins: [react()],
  test: {
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
    css: true,
    /**
     * Vitest's own default is 5 s, the *same* number `src/test/setup.ts` gives a single
     * `waitFor` — so one slow wait could eat a test's whole budget and the failure read as
     * "test timed out" rather than as the load it was. Both numbers are load thresholds and
     * not behaviour ones: this workstation runs the app and twenty streaming phones beside the
     * suite, and specs that pass alone were failing in the full run, a different one each time.
     */
    testTimeout: 20_000,
    exclude: [...configDefaults.exclude, "e2e/**"],
  },
});
