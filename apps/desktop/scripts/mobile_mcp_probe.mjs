#!/usr/bin/env node

import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const desktopRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repositoryRoot = resolve(desktopRoot, "..", "..");
const wrapper = resolve(repositoryRoot, "scripts", "run_mobile_mcp.mjs");
const includeDevices = process.argv.includes("--devices");
const expectedTools = [
  "mobile_list_available_devices",
  "mobile_take_screenshot",
  "mobile_list_elements_on_screen",
  "mobile_click_on_screen_at_coordinates",
  "mobile_swipe_on_screen",
  "mobile_start_screen_recording",
  "mobile_stop_screen_recording",
  "mobile_list_crashes",
  "mobile_get_crash",
];

const transport = new StdioClientTransport({
  command: process.execPath,
  args: [wrapper],
  cwd: repositoryRoot,
  env: { ...process.env },
  stderr: "pipe",
});
let stderr = "";
transport.stderr?.on("data", (chunk) => {
  stderr += chunk.toString();
});

const client = new Client({ name: "riviu-mobile-mcp-probe", version: "1.0.0" });

try {
  await client.connect(transport);
  const listed = await client.listTools(undefined, { timeout: 30_000 });
  const names = listed.tools.map(({ name }) => name).sort();
  const missing = expectedTools.filter((name) => !names.includes(name));
  if (missing.length > 0) {
    throw new Error(`mobile-mcp is missing reviewed tools: ${missing.join(", ")}`);
  }

  let devices = null;
  if (includeDevices) {
    const result = await client.callTool(
      { name: "mobile_list_available_devices", arguments: {} },
      undefined,
      { timeout: 45_000 },
    );
    if (result.isError) throw new Error("mobile_list_available_devices returned an MCP error");
    devices = result.content;
  }

  console.log(
    JSON.stringify(
      {
        status: "pass",
        transport: "stdio",
        toolCount: names.length,
        expectedTools,
        devices,
      },
      null,
      2,
    ),
  );
} catch (error) {
  if (stderr.trim()) console.error(stderr.trim());
  throw error;
} finally {
  await client.close();
}
