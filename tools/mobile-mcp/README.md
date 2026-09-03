# Mobile MCP canary tool

Riviu pins `@mobilenext/mobile-mcp` `1.0.2` as a development-only tool. It is for
accessibility-tree inspection, screenshots, recordings and disposable-device smoke
tests. It is not a `DeviceDriver`, is not started by the desktop app and is not shipped
inside MSI/NSIS.

## Install and verify

```powershell
npm ci --prefix apps/desktop
npm --prefix apps/desktop run mobile-mcp:check -- --require-adb
npm --prefix apps/desktop run test:mobile-mcp
npm --prefix apps/desktop run mobile-mcp:probe
```

The launcher follows Riviu's development adb precedence:
`RIVIU_ADB_PATH -> ANDROID_SDK_ROOT -> ANDROID_HOME -> PATH -> bundled`. It copies
the selected executable and adjacent Windows DLLs to an immutable, content-addressed
SDK shim under `target/mobile-mcp/`, then forces Mobile MCP to use that shim. An adb
server can keep the selected executable open without blocking the next probe.

Start the stdio server through the launcher rather than `npx`:

```powershell
npm --prefix apps/desktop run mobile-mcp
```

The wrapper pins the installed package and integrity, disables Mobile MCP and
Mobilewright telemetry, disables unsafe URL schemes, selects the legacy local robot and
rejects `--listen`. The repository never uses `@latest`.

For a local read-only device enumeration after the protocol probe passes:

```powershell
npm --prefix apps/desktop run mobile-mcp:probe -- --devices
```

## Operating boundary

- Stop Riviu Manager before using the same device through Mobile MCP.
- Use one disposable Android canary. Do not select a production fleet device.
- Read-only inspection is the default workflow. A tap, type, install or app-state change
  is a direct device effect outside Riviu's lease, audit and reconciliation controls.
- Do not use Mobile MCP for TikTok Like, Save, Comment, Follow or Post. Capture a fixture,
  encode the locator in Riviu and run it through `DeviceControlPlane` instead.
- Do not use the remote/cloud tools. This integration is local-only.

`mobile-mcp` requires Node.js 20 or newer. Node is a source-development prerequisite;
the Riviu installer does not need Node on the destination machine.
