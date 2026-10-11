import { defineConfig, devices } from "@playwright/test";

// Every worktree defaults to the same port, and `reuseExistingServer` would then
// silently test whichever worktree's dev server holds it. Give each parallel
// worktree its own RIVIU_E2E_PORT; CI and a lone checkout keep 1421.
const port = Number(process.env.RIVIU_E2E_PORT ?? 1421);
if (!Number.isInteger(port) || port < 1024 || port > 65535) {
  throw new Error(`RIVIU_E2E_PORT must be an integer port in 1024-65535, got ${process.env.RIVIU_E2E_PORT}`);
}
const origin = `http://127.0.0.1:${port}`;

export default defineConfig({
  testDir: "./e2e",
  outputDir: "./node_modules/.tmp/playwright-results",
  fullyParallel: false,
  retries: 0,
  timeout: 30_000,
  expect: { timeout: 5_000 },
  use: {
    baseURL: origin,
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  webServer: {
    command: `npm run dev -- --host 127.0.0.1 --port ${port} --strictPort`,
    url: origin,
    reuseExistingServer: !process.env.CI,
    timeout: 60_000,
  },
});
