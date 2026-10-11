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
    /**
     * Half the cores, not one worker per core. The first case of each heavy jsdom file
     * (`css: true`) pays the file's cold start, and with 19 workers on this 20-thread host it
     * reached 20.4 s (PublishPage) and 18.1 s (App) and timed out in a full run. Measured
     * 12/10/2026 on the idle host: 233 s wall at the default, 225 s at 50% with those cases
     * at 7.6 s and 14 s and no failure.
     */
    maxWorkers: "50%",
    exclude: [...configDefaults.exclude, "e2e/**"],
  },
});
