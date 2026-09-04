import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AnalyticsSummary,
  AppEvent,
  AgentRuntimeView,
  AgentSettings,
  AgentStatus,
  AppLibraryItem,
  AppInstallBatchResponse,
  AppInstallRequest,
  AppleIdConfig,
  ClipboardRead,
  DeviceDirListing,
  DeviceGroup,
  DeviceInfo,
  DeviceWorkState,
  GroupInputReport,
  GroupSyncPolicy,
  DeviceMeta,
  GroupInstallResult,
  HardwareKey,
  ActionDefinition,
  CompiledRevision,
  FlowArtifactPayload,
  FlowCoordinateFrame,
  FlowDocumentV2,
  FlowNodeAttemptRecord,
  FlowRevisionRecord,
  FlowRunDetail,
  FlowRunRecord,
  FlowSummary,
  FlowTargetSelection,
  LegacyImportResult,
  JobRecord,
  MaterialItem,
  MaterialPushBatchRequest,
  MaterialPushBatchResult,
  PublishCampaignDetail,
  PublishCampaignExecutionResult,
  PublishCampaignRecord,
  PublishCaptionOverrides,
  PublishExecutionSnapshot,
  PublishAssignmentPlan,
  PublishFolderManifest,
  PublishPreflightReport,
  PublishPreflightRequest,
  PublishSoundPolicy,
  PublicCleanupCapability,
  PublicCleanupExecutionReport,
  PublicCleanupKind,
  ScheduleItem,
  StreamSettings,
  NurtureApiTestResult,
  NurtureCommentAttempt,
  NurtureCostSummary,
  OpLog,
  NurtureSessionStatus,
  OperationRunDetail,
  OperationRunSummary,
  SessionLogEntry,
  SessionLogSummary,
  NurtureSettings,
  InteractionCampaignDetail,
  InteractionCampaignSummary,
  InteractionTargetNote,
  ThreadCampaignRequest,
  ThreadPreview,
  InteractionPostReading,
  PostTargets,
  ResolvedTikTokTarget,
  TikTokLinkLine,
  InstalledApp,
  ShellOutcome,
  UpdateStatus,
  DeviceRootStatus,
  DeviceHealthReport,
  DevicePublishReadiness,
  PublishSheetConfig,
  AutomationDefinition,
  AutomationDefinitionRecord,
  AutomationKind,
  AutomationSchedule,
  AutomationScheduleV1,
  JsonValue,
  CompiledOrchestrationV1,
  OrchestrationDocumentV1,
  OrchestrationRunDetail,
  OrchestrationRunRecord,
  OrchestrationRevisionRecord,
  OrchestrationSummary,
  TargetRef,
} from "./types";
import { asAppEvent } from "./types";

export async function startupError() {
  return invoke<string | null>("startup_error");
}

/**
 * Run the bootstrap again and report what is wrong *now*.
 *
 * `null` means the app came up. The startup screen's button used to call
 * `window.location.reload()`, which reloaded the WebView and got the same stored sentence
 * back — the bootstrap had run once at setup and would never run again, so fixing the cause
 * (plugging in adb, starting the sidecar) had no way to reach the app short of quitting it.
 */
export async function retryStartup() {
  return invoke<string | null>("retry_startup");
}

export async function listDevices() {
  return invoke<DeviceInfo[]>("list_devices");
}

export async function listDeviceWorkStates() {
  return invoke<DeviceWorkState[]>("list_device_work_states");
}

export async function refreshDevices() {
  return invoke<DeviceInfo[]>("refresh_devices");
}

export async function prepareDevice(udid: string) {
  return invoke<DeviceInfo>("prepare_device", { udid });
}

export async function installIpa(udid: string, path: string) {
  return invoke<void>("install_ipa", { udid, path });
}

export async function installIpaToGroup(groupId: string, path: string) {
  return invoke<GroupInstallResult[]>("install_ipa_to_group", { groupId, path });
}

export async function screenshot(udid: string) {
  return invoke<string>("screenshot", { udid });
}

export async function rebootDevice(udid: string) {
  return invoke<void>("reboot_device", { udid });
}

export async function backupDevice(udid: string, dest: string) {
  return invoke<void>("backup_device", { udid, dest });
}

export async function restoreDevice(udid: string, src: string) {
  return invoke<void>("restore_device", { udid, src });
}

export async function deviceTap(
  udid: string,
  x: number,
  y: number,
  imageW?: number,
  imageH?: number,
) {
  return invoke<void>("device_tap", {
    udid,
    x,
    y,
    imageW: imageW ?? null,
    imageH: imageH ?? null,
  });
}

export interface SwipeSample {
  x: number;
  y: number;
  /// Milliseconds since the previous sample. The agent gives each `pointerMove` its own
  /// duration, so this is what carries the gesture's velocity -- a drag that starts slow
  /// and eases out reaches the phone as exactly that.
  durationMs: number;
}

/// A drag as the path the finger took, in encoded-frame pixels.
///
/// `deviceSwipe` sends two endpoints, which the framework receives as a straight line at
/// constant speed. This sends the samples, and the whole curve goes in ONE round trip
/// because the agent's `/actions` accepts any number of moves.
export async function deviceSwipePath(
  udid: string,
  start: { x: number; y: number },
  steps: SwipeSample[],
  imageW: number,
  imageH: number,
  settleMs = 40,
) {
  return invoke<void>("device_swipe_path", {
    udid,
    path: {
      start,
      steps: steps.map((step) => ({
        point: { x: step.x, y: step.y },
        durationMs: Math.max(1, Math.round(step.durationMs)),
      })),
      settleMs,
    },
    imageW,
    imageH,
  });
}

export async function deviceSwipe(
  udid: string,
  fromX: number,
  fromY: number,
  toX: number,
  toY: number,
  imageW?: number,
  imageH?: number,
  durationMs = 280,
) {
  return invoke<void>("device_swipe", {
    udid,
    gesture: {
      from: { x: fromX, y: fromY },
      to: { x: toX, y: toY },
      durationMs,
    },
    imageW: imageW ?? null,
    imageH: imageH ?? null,
  });
}

export async function deviceTypeText(udid: string, text: string) {
  return invoke<void>("device_type_text", { udid, text });
}

export async function deviceKey(udid: string, key: HardwareKey) {
  return invoke<void>("device_key", { udid, key });
}

/// Lock (screen off) or unlock a phone (D). iOS via WDA; Android sleeps/wakes. A phone with
/// a secure PIN wakes to its own lock screen — this is a batch screen on/off, not a bypass.
export async function setScreenLocked(udid: string, locked: boolean) {
  return invoke<void>("set_screen_locked", { udid, locked });
}

export async function deviceControlBegin(udid: string) {
  return invoke<void>("device_control_begin", { udid });
}

export async function deviceControlEnd(udid: string) {
  return invoke<void>("device_control_end", { udid });
}

export async function groupInput(payload: {
  udids: string[];
  kind: string;
  x?: number;
  y?: number;
  toX?: number;
  toY?: number;
  text?: string;
  imageW?: number;
  imageH?: number;
  key?: HardwareKey;
  sync?: GroupSyncPolicy;
}) {
  // Was `invoke<void>`, which threw away the report the command has always returned. A
  // twenty-phone action that reached zero of them resolved as a success and toasted nothing.
  return invoke<GroupInputReport>("group_input", {
    udids: payload.udids,
    kind: payload.kind,
    x: payload.x ?? null,
    y: payload.y ?? null,
    toX: payload.toX ?? null,
    toY: payload.toY ?? null,
    text: payload.text ?? null,
    imageW: payload.imageW ?? null,
    imageH: payload.imageH ?? null,
    key: payload.key ?? null,
    sync: payload.sync ?? null,
  });
}

/// Type a different string onto each phone (A2, "Text Distribution"). `assignments` is
/// already paired to phones in the operator's order; returns the same per-device report as
/// group input so partial success is visible.
export async function distributeText(assignments: { udid: string; text: string }[]) {
  return invoke<GroupInputReport>("distribute_text", { assignments });
}

/// Push a different file into each phone's gallery (A2, "File Distribution"). `assignments`
/// pairs each phone to a local path in the operator's order; returns a per-device report.
export async function distributeFiles(assignments: { udid: string; path: string }[]) {
  return invoke<GroupInputReport>("distribute_files", { assignments });
}

/// Put a USB Android phone into wireless adb and connect (A4). Returns `host:port`.
export async function enableWifiAdb(udid: string) {
  return invoke<string>("enable_wifi_adb", { udid });
}

/// Put adbd back on USB, closing the `0.0.0.0:5555` port (A4).
///
/// Not the same as `wifiAdbDisconnect`, which only drops this host's client and leaves the
/// phone listening to the whole LAN.
export async function disableWifiAdb(udid: string) {
  return invoke<void>("disable_wifi_adb", { udid });
}

/// Manually `adb connect host:port` (A4).
export async function wifiAdbConnect(host: string) {
  return invoke<void>("wifi_adb_connect", { host });
}

/// `adb disconnect host:port` (A4).
export async function wifiAdbDisconnect(host: string) {
  return invoke<void>("wifi_adb_disconnect", { host });
}

export interface ArpEntry {
  ip: string;
  mac: string;
}

/// Scan the host ARP table for LAN devices to connect wirelessly (A9).
export async function arpScan() {
  return invoke<ArpEntry[]>("arp_scan");
}

/// Set an Android phone's wallpaper from a local image file (A3).
export async function setWallpaper(udid: string, path: string) {
  return invoke<void>("set_wallpaper", { udid, path });
}

/// Set an Android wallpaper from PNG bytes rendered in the webview (A3, number wallpaper).
export async function setWallpaperBytes(udid: string, png: number[]) {
  return invoke<void>("set_wallpaper_bytes", { udid, png });
}

/// Inject a mock GPS location on an Android phone (B). Needs the Riviu helper + the
/// mock-location appop (the backend grants it best-effort).
export async function setMockLocation(udid: string, lat: number, lng: number) {
  return invoke<void>("set_mock_location", { udid, lat, lng });
}

/// Stop mock location, returning the phone to real GPS (B).
export async function stopMockLocation(udid: string) {
  return invoke<void>("stop_mock_location", { udid });
}

// --- Root tier (C, xiaowei "ROOT 模式"). Two different privilege routes, and this fleet
// disagrees on nine of twenty phones -- see `DeviceRootStatus`. ---

/// The two separate answers to "is this phone rooted": `hasSu`, and `shellIsRoot`.
///
/// Used to return a single boolean meaning `hasSu`, which reported nine phones as unrooted
/// while their adb shell was already uid 0. `factory_reset` is still gated on `hasSu`
/// specifically, so the panel has to show both rather than collapse them.
export async function isRooted(udid: string) {
  return invoke<DeviceRootStatus>("is_rooted", { udid });
}

/** One phone's health, section by section — read-only, takes no lease. */
export async function deviceHealth(udid: string) {
  return invoke<DeviceHealthReport>("device_health", { udid });
}

/// Overwrite the app-visible device fingerprint (C, xiaowei 一键新机). `androidId` applies
/// without root; `serialno`/`mac` need root. Returns a summary of what changed per field.
export async function setDeviceIdentity(
  udid: string,
  identity: { androidId?: string; serialno?: string; mac?: string },
) {
  return invoke<string>("set_device_identity", {
    udid,
    androidId: identity.androidId ?? null,
    serialno: identity.serialno ?? null,
    mac: identity.mac ?? null,
  });
}

/// Factory-reset a rooted Android phone (C). Irreversible — callers confirm first.
export async function factoryReset(udid: string) {
  return invoke<void>("factory_reset", { udid });
}

/// Run one root shell command on a rooted Android phone (C, advanced). Errors if not rooted.
export async function rootShell(udid: string, command: string) {
  return invoke<string>("root_shell", { udid, command });
}

// --- The per-phone function menu (xiaowei 功能). Every row of that menu is one command
// here, and none of them assembles a shell string in TypeScript: the path and package
// validators live in Rust, and a menu item that skirts them is a menu item with no
// validator at all. ---

/// Read one directory on the phone, for the file browser (xiaowei "Preview Mobile Files").
/// `path` must be absolute; the backend refuses anything a single quote could break out of.
export async function deviceListDir(udid: string, path: string) {
  return invoke<DeviceDirListing>("device_list_dir", { udid, path });
}

/// Copy one file or folder off the phone (xiaowei "Export File"). Returns the local path.
export async function devicePullPath(udid: string, remote: string, destDir: string) {
  return invoke<string>("device_pull_path", { udid, remote, destDir });
}

/// Put one local file onto the phone (xiaowei "Import File"). Returns the device path.
export async function devicePushFile(udid: string, local: string, remoteDir: string) {
  return invoke<string>("device_push_file", { udid, local, remoteDir });
}

/// Delete a file or folder on the phone. The backend refuses storage roots outright; every
/// other target is confirmed by the caller first.
export async function deviceDeletePath(udid: string, path: string) {
  return invoke<void>("device_delete_path", { udid, path });
}

/// Turn the phone's own Wi-Fi radio on or off, returning the state it settled at (xiaowei
/// ADB submenu). Not this app's wireless-adb link — a phone reached over Wi-Fi drops itself.
export async function setWifiRadio(udid: string, on: boolean) {
  return invoke<boolean>("set_wifi_radio", { udid, on });
}

/// Put the display back to factory density and/or resolution (xiaowei "Reset DPI" / "Reset
/// resolution"). Returns the phone's own reading afterwards, to show rather than claim.
export async function resetDisplayMetrics(udid: string, density: boolean, size: boolean) {
  return invoke<string>("reset_display_metrics", { udid, density, size });
}

/// Power the phone off (xiaowei "Shutdown"). Only a human at the phone can undo it.
export async function powerOffDevice(udid: string) {
  return invoke<void>("power_off_device", { udid });
}

/// Open the phone's own Settings app (xiaowei "Phone Settings").
export async function openSystemSettings(udid: string) {
  return invoke<void>("open_system_settings", { udid });
}

/// Wake the screen (xiaowei "Turn On Screen"). Idempotent — it cannot put a phone to sleep.
export async function wakeScreen(udid: string) {
  return invoke<void>("wake_screen", { udid });
}

/// Screenshot into the phone's own gallery (xiaowei "Screenshot to phone"). Returns the
/// device path. `screenshot` above is the other row: that one copies to this machine.
export async function screenshotToDevice(udid: string) {
  return invoke<string>("screenshot_to_device", { udid });
}

/// Switch the phone's keyboard (xiaowei "Switch Input Method"). Only pass an id the phone
/// itself printed — see `imeList.ts` for why the list is parsed rather than composed.
export async function setInputMethod(udid: string, imeId: string) {
  return invoke<void>("set_input_method", { udid, imeId });
}

/// Start one app on the phone (xiaowei's App List, where a row click launches).
export async function launchDeviceApp(udid: string, bundleId: string) {
  return invoke<void>("launch_device_app", { udid, bundleId });
}

/// Read the phone's clipboard onto this machine (xiaowei "Export Clipboard").
export async function deviceGetClipboard(udid: string) {
  return invoke<ClipboardRead>("device_get_clipboard", { udid });
}

/// Write text onto the phone's clipboard, for the operator to paste there by hand.
export async function deviceSetClipboard(udid: string, text: string) {
  return invoke<void>("device_set_clipboard", { udid, text });
}

export async function getStreamSettings() {
  return invoke<StreamSettings>("get_stream_settings");
}

export async function setStreamSettings(settings: StreamSettings) {
  return invoke<StreamSettings>("set_stream_settings", { settings });
}

/// Local automation API (B, xiaowei "openapi"). Loopback-only HTTP server, off by default.
export interface LocalApiConfig {
  enabled: boolean;
  port: number;
  token: string;
}

export async function localApiGetConfig() {
  return invoke<LocalApiConfig>("local_api_get_config");
}

/// Persist the config. The server binds at startup, so a change applies on next app launch.
/// Enabling without a token makes the backend mint one; the returned config carries it.
export async function localApiSetConfig(config: LocalApiConfig) {
  return invoke<LocalApiConfig>("local_api_set_config", { config });
}

/// USB relay (D peripherals, xiaowei "外设"). A host serial port the relay board is on.
export interface SerialPortInfo {
  name: string;
  kind: string;
}

/// List host serial ports, to pick the relay board's COM port (D).
export async function listSerialPorts() {
  return invoke<SerialPortInfo[]>("list_serial_ports");
}

/// Hold a relay channel on/off (D). Raw state.
export async function relaySetChannel(port: string, channel: number, on: boolean) {
  return invoke<void>("relay_set_channel", { port, channel, on });
}

/// Pulse a relay channel and return it — the hard reboot (D). `energize=true` presses (on→off),
/// `false` cuts power (off→on). `holdMs` is clamped 50–10000 by the backend.
export async function relayPulseChannel(
  port: string,
  channel: number,
  holdMs: number,
  energize: boolean,
) {
  return invoke<void>("relay_pulse_channel", { port, channel, holdMs, energize });
}

export async function viewEndpoint() {
  return invoke<string | null>("view_endpoint");
}

export async function viewEnsure(udid: string) {
  return invoke<void>("view_ensure", { udid });
}

/** One device's decoder counters, as the host's watchdog needs them. */
export interface ViewPaintReport {
  udid: string;
  /** Which producer these counters belong to. Evidence about a replaced one is dropped. */
  generation: number;
  received: number;
  frames: number;
  /** Envelopes received since the last frame; isolated codec packets are not a video stall. */
  packetsSincePaint: number;
  /**
   * Age of the last drawn frame, in ms, by *this* clock.
   *
   * An age rather than a timestamp on purpose: the WebView and the host keep different
   * clocks, and comparing them across the IPC boundary turns a sleeping laptop into a
   * fleet-wide restart.
   */
  sincePaintMs: number;
}

export async function viewReportPaint(reports: ViewPaintReport[]) {
  return invoke<void>("view_report_paint", { reports });
}

/**
 * Ask the phone for a fresh keyframe without restarting its producer.
 *
 * Returns false when there is no producer to ask, which is not a failure.
 */
export async function viewRequestKeyframe(udid: string) {
  return invoke<boolean>("view_request_keyframe", { udid });
}

export type TouchAction = "down" | "move" | "up";

/// One live touch on the scrcpy control socket. Resolves false when the phone is not
/// streaming, which means the caller should fall back to the agent rather than report an error.
export async function viewInjectTouch(
  udid: string,
  action: TouchAction,
  x: number,
  y: number,
  imageW: number,
  imageH: number,
) {
  return invoke<boolean>("view_inject_touch", {
    udid,
    action,
    x,
    y,
    imageW,
    imageH,
  });
}

export async function viewSetPreset(udid: string, preset: "tile" | "overlay") {
  return invoke<void>("view_set_preset", { udid, preset });
}

export async function saveViewSnapshot(udid: string, jpeg: number[]) {
  return invoke<string>("save_view_snapshot", { udid, jpeg });
}

export async function listJobs() {
  return invoke<JobRecord[]>("list_jobs");
}

export async function operationListRuns(limit = 100) {
  return invoke<OperationRunSummary[]>("operation_list_runs", { limit });
}

export async function operationGetRun(operationId: string) {
  return invoke<OperationRunDetail | null>("operation_get_run", { operationId });
}

export async function runScript(scriptJson: string, udids: string[]) {
  return invoke<JobRecord>("run_script", { scriptJson, udids });
}

export async function cancelJob(jobId: string) {
  return invoke<void>("cancel_job", { jobId });
}

export async function listScripts() {
  return invoke<[string, string][]>("list_scripts");
}

export async function saveScript(name: string, bodyJson: string) {
  return invoke<void>("save_script", { name, bodyJson });
}

export async function exampleScript() {
  return invoke<string>("example_script");
}

export async function getAppleId() {
  return invoke<AppleIdConfig>("get_apple_id");
}

export async function setAppleId(email: string, password: string) {
  return invoke<void>("set_apple_id", { email, password });
}

export async function clearAppleId() {
  return invoke<void>("clear_apple_id");
}

export async function agentGetSettings() {
  return invoke<AgentRuntimeView>("agent_get_settings");
}

export async function agentSaveSettings(settings: AgentSettings) {
  return invoke<AgentRuntimeView>("agent_save_settings", { settings });
}

export async function agentListStatuses(udids: string[]) {
  return invoke<AgentStatus[]>("agent_list_statuses", { udids });
}

export async function agentPreflight(udid: string) {
  return invoke<AgentStatus>("agent_preflight", { udid });
}

export async function agentRepair(udid: string) {
  return invoke<AgentStatus>("agent_repair", { udid });
}

export async function agentBulkRepair(udids: string[]) {
  return invoke<AgentStatus[]>("agent_bulk_repair", { udids });
}

export async function driverMode() {
  return invoke<string>("driver_mode");
}

/** Why real devices cannot be listed, or null when the sidecar is healthy. */
export async function driverDegradedReason() {
  return invoke<string | null>("driver_degraded_reason");
}

/**
 * Why the Android half of the fleet is absent, or null when it joined.
 *
 * Deliberately not folded into `driverDegradedReason`: "this machine has no adb" and
 * "the iOS sidecar failed" are different facts with different fixes. The command existed
 * and was registered from the start; nothing on this side ever called it, so an Android
 * phone simply did not appear and said nothing about why.
 */
/**
 * What went wrong verifying the bundled Android tools at boot. Empty means healthy.
 *
 * Distinct from `androidUnavailableReason` on purpose: that one means no Android phone can join
 * the fleet, this one means they join and then cannot be driven — which is the shape of the bug
 * reported from a real install ("nhận điện thoại rồi, nhưng điều khiển không được").
 */
/**
 * Write a frontend failure into the app log.
 *
 * **Never rejects, on purpose.** This is called from the unhandled-rejection handler, so a
 * rejecting promise here would raise a new unhandled rejection and report itself forever. If
 * the bridge is unavailable there is nowhere better to send the failure than nowhere, and a
 * silent drop is strictly better than a loop.
 */
export async function logFrontendError(
  kind: string,
  message: string,
  source?: string,
): Promise<void> {
  try {
    await invoke<void>("log_frontend_error", { kind, message, source: source ?? null });
  } catch {
    // Deliberately empty: see above.
  }
}

export async function androidToolProblems() {
  return invoke<string[]>("android_tool_problems");
}

export async function androidUnavailableReason() {
  return invoke<string | null>("android_unavailable_reason");
}

/** Complete the installed-package smoke only after the React fleet has settled. */
export async function deploymentFrontendReady() {
  return invoke<boolean>("deployment_frontend_ready");
}

/** Log folder derived by Tauri from the identifier of the running build. */
export async function appLogDirectory() {
  return invoke<string>("app_log_directory");
}

/**
 * Ask GitHub whether a newer release exists, and whether taking it now is safe.
 *
 * Never called on mount. A farm machine is often offline and nobody asked it to phone
 * home, so this is only ever a press.
 */
export async function updateCheck() {
  return invoke<UpdateStatus>("update_check");
}

/**
 * Download the update and hand over to its installer.
 *
 * On Windows the process is replaced from under us, so this promise never settles on
 * success — treat a resolve as "unpacked, reopen the app" and a reject as the reason.
 * The backend re-checks `busyReason` itself; the disabled button here is courtesy, not
 * the guard.
 */
export async function updateInstall() {
  return invoke<void>("update_install");
}

/**
 * Every app one phone reports as present.
 *
 * One udid per call on purpose. The backend has no batch form, because a batch would
 * paint nothing until the slowest phone in the fleet answered; callers fan out and each
 * row appears when its own phone replies.
 *
 * Rejects rather than returning `[]` when a backend cannot enumerate — an empty array
 * would read as a phone with nothing installed. Show the rejection text.
 */
export async function listInstalledApps(udid: string) {
  return invoke<InstalledApp[]>("list_installed_apps", { udid });
}

/**
 * Run one operator-typed shell command on a device.
 *
 * `adb shell <script>` only. The backend has no path to `adb <subcommand>`, so install,
 * reboot, root and kill-server are not one typo away from a text box.
 */
/** Put one picture or video into the phone's gallery, where it is actually visible. */
export async function importMedia(udid: string, path: string) {
  return invoke<string>("import_media", { udid, path });
}

/// What an export found on the phone, and what of it reached this machine.
export interface MediaExportReport {
  fetched: number;
  found: number;
  /// Found on the phone and did not arrive. Zero on a healthy export.
  missed: number;
}

/**
 * Copy the phone's photos and videos into `destDir`.
 *
 * Both counts, because `fetched` alone cannot express the failure that matters: a phone
 * with five hundred photos of which twenty copied returns the same `20` as a phone that
 * only ever had twenty, and the second is the one where nothing is wrong.
 */
export async function exportMedia(udid: string, destDir: string) {
  return invoke<MediaExportReport>("export_media", { udid, destDir });
}

export async function deviceShell(udid: string, script: string) {
  return invoke<ShellOutcome>("device_shell", { udid, script });
}

/**
 * Ask a device to rotate, and get back the rotation it ACTUALLY settled at.
 *
 * The returned value is frequently not the one requested: a portrait-locked foreground
 * app wins, which on this farm is most of the time. Compare before telling the operator
 * anything happened.
 */
export async function setScreenRotation(udid: string, rotation: 0 | 1 | 2 | 3) {
  return invoke<number>("set_screen_rotation", { udid, rotation });
}

export async function getDeviceMeta(udid: string) {
  return invoke<DeviceMeta>("get_device_meta", { udid });
}

/// Every phone this app has a record for, in one call — what the grid reads to label and
/// order tiles. Phones nobody has edited have no row, so an untouched fleet answers empty.
export async function listDeviceMetas() {
  return invoke<DeviceMeta[]>("list_device_metas");
}

export async function saveDeviceMeta(meta: DeviceMeta) {
  return invoke<void>("save_device_meta", { meta });
}

export async function listGroups() {
  return invoke<DeviceGroup[]>("list_groups");
}

export async function saveGroup(group: DeviceGroup) {
  return invoke<DeviceGroup>("save_group", { group });
}

/**
 * Delete a group and its membership.
 *
 * The phones are not touched — a group is a label this app keeps, so deleting one returns its
 * phones to "chưa thuộc nhóm nào" and nothing else changes.
 */
export async function deleteGroup(id: string) {
  return invoke<void>("delete_group", { id });
}

export async function listMaterials() {
  return invoke<MaterialItem[]>("list_materials");
}

export async function addMaterial(sourcePath: string, name?: string) {
  return invoke<MaterialItem>("add_material", { sourcePath, name: name ?? null });
}

export async function deleteMaterial(id: string) {
  return invoke<void>("delete_material", { id });
}

export async function pushMaterial(udid: string, materialId: string) {
  return invoke<string>("push_material", { udid, materialId });
}

export async function pushMaterialBatch(request: MaterialPushBatchRequest) {
  return invoke<MaterialPushBatchResult>("push_material_batch", { request });
}

export async function listAppsLibrary() {
  return invoke<AppLibraryItem[]>("list_apps_library");
}

export async function addAppLibrary(
  sourcePath: string,
  name?: string,
  bundleId?: string,
  version?: string,
) {
  return invoke<AppLibraryItem>("add_app_library", {
    sourcePath,
    name: name ?? null,
    bundleId: bundleId ?? null,
    version: version ?? null,
  });
}

export async function deleteAppLibrary(id: string) {
  return invoke<void>("delete_app_library", { id });
}

export async function installLibraryApp(udid: string, appId: string) {
  return invoke<void>("install_library_app", { udid, appId });
}

export async function installLibraryAppToGroup(groupId: string, appId: string) {
  return invoke<GroupInstallResult[]>("install_library_app_to_group", { groupId, appId });
}

export async function installLibraryAppBatch(request: AppInstallRequest) {
  return invoke<AppInstallBatchResponse>("install_library_app_batch", { request });
}

export async function cancelAppInstallBatch(batchId: string) {
  return invoke<void>("cancel_app_install_batch", { batchId });
}

export async function listSchedules() {
  return invoke<ScheduleItem[]>("list_schedules");
}

export async function saveSchedule(schedule: ScheduleItem) {
  return invoke<ScheduleItem>("save_schedule", { schedule });
}

export async function deleteSchedule(id: string) {
  return invoke<void>("delete_schedule", { id });
}

/**
 * Deal not-yet-published bundles onto the selected phones.
 *
 * Returns the pairing for the operator to look at; it creates nothing. The pool is what the
 * database says has not been dispatched, so pressing this twice does not re-deal the same
 * posts — see `auto_assign_bundles`.
 */
export async function publishAutoAssign(sourceRoot: string, udids: string[], wanted: number) {
  return invoke<{ plan: PublishAssignmentPlan[] }>("publish_auto_assign", {
    sourceRoot,
    udids,
    wanted,
  });
}

export async function publishScanFolder(sourceRoot: string) {
  return invoke<PublishFolderManifest>("publish_scan_folder", { sourceRoot });
}

/** Read-only validation of the exact input that will be allowed to create a campaign. */
export async function publishPreflight(request: PublishPreflightRequest) {
  return invoke<PublishPreflightReport>("publish_preflight", { request });
}

export async function publishCreateCampaign(
  sourceRoot: string,
  bundleIds: string[],
  udids: string[],
  runAt: string | null,
  captionOverrides: PublishCaptionOverrides | null,
  soundPolicy: PublishSoundPolicy,
  targetRef: TargetRef,
  confirmed: boolean,
  approvedInputDigest: string,
) {
  return invoke<PublishCampaignRecord>("publish_create_campaign", {
    sourceRoot,
    bundleIds,
    udids,
    runAt: runAt ?? null,
    captionOverrides: captionOverrides ?? null,
    soundPolicy,
    targetRef,
    confirmed,
    approvedInputDigest,
  });
}

export async function publishList(limit = 50) {
  return invoke<PublishCampaignRecord[]>("publish_list", { limit });
}

export async function publishCancel(campaignId: string) {
  return invoke<void>("publish_cancel", { campaignId });
}

/**
 * One operator confirmation for preflight through Sheet completion.
 *
 * A typed partial response means no public retry should be inferred from an exception. When a
 * post is already confirmed, the backend limits retry to link capture and/or the idempotent
 * Sheet outbox.
 */
export async function publishExecute(
  campaignId: string,
  confirmed: boolean,
) {
  return invoke<PublishCampaignExecutionResult>("publish_execute", {
    campaignId,
    confirmed,
  });
}

/** Why each phone can or cannot take the publish route — the preflight's answer, before the refusal. */
export async function publishReadiness(udids: string[]) {
  return invoke<DevicePublishReadiness[]>("publish_readiness", { udids });
}

/** One campaign with its bundles, per-phone assignments and event history. */
export async function publishGet(campaignId: string) {
  return invoke<PublishCampaignDetail | null>("publish_get", { campaignId });
}

/** Rebuild the retry boundary from durable state before an operator resumes a campaign. */
export async function publishReconcile(campaignId: string) {
  return invoke<PublishExecutionSnapshot>("publish_reconcile", { campaignId });
}

/** The Sheet delivery config, minus the token itself (only whether one is set). */
export async function publishSheetGetConfig() {
  return invoke<PublishSheetConfig>("publish_sheet_get_config");
}

/**
 * Save the webhook URL; `token` undefined keeps the stored one, an empty string clears it.
 * The backend refuses a non-HTTPS URL — the token and every post link travel in the body.
 */
export async function publishSheetSaveConfig(webhookUrl: string, token?: string) {
  return invoke<PublishSheetConfig>("publish_sheet_save_config", {
    webhookUrl,
    token: token ?? null,
  });
}

/**
 * The operation log, deeper than the twenty rows `analytics_summary` bundles in.
 *
 * Registered and allowlisted since the farm pages landed, and called by nothing — while
 * `log_op` wrote to the table from fifteen places. See `OperationLog`.
 */
/**
 * The last `lines` of the phone's own log.
 *
 * Registered with `Driver::syslog_tail` behind it and seven test mocks stubbing it, and called
 * by nothing — so the app that drives the phone could not read the phone's log. Takes the lease
 * with `LeaseStream::Park`, which is why `DeviceSyslogPopup` warns that the tile goes quiet.
 */
export async function syslog(udid: string, lines?: number) {
  return invoke<string>("syslog", { udid, lines });
}

export async function listOpLogs(limit?: number) {
  return invoke<OpLog[]>("list_op_logs", { limit });
}

export async function analyticsSummary() {
  return invoke<AnalyticsSummary>("analytics_summary");
}

export async function apiDocs() {
  return invoke<string>("api_docs");
}

export async function nurtureGetSettings() {
  return invoke<NurtureSettings>("nurture_get_settings");
}

export async function nurtureSaveSettings(settings: NurtureSettings) {
  return invoke<NurtureSettings>("nurture_save_settings", { settings });
}

/// Draft one comment from what this device is showing, without sending anything.
///
/// `frames` carries the pictures the WebView has already decoded. Devices on the H.264
/// view path — every Android phone — publish nothing into the host's JPEG hub, so without
/// this the command answered "no frames" about a phone whose live picture was on screen.
export async function nurtureTestApi(udid: string, frames?: Uint8Array[]) {
  return invoke<NurtureApiTestResult>("nurture_test_api", {
    udid,
    frames: frames?.length ? frames.map((frame) => Array.from(frame)) : null,
  });
}

export async function nurtureSessionStatus() {
  return invoke<NurtureSessionStatus[]>("nurture_session_status");
}

/// One device's history, oldest line first.
///
/// Fetched per device rather than pushed with the status stream: the statuses go to every
/// row continuously, and hanging two hundred lines off each one would multiply that by the
/// fleet size for a panel that shows one phone at a time.
export async function nurtureSessionLog(udid: string) {
  return invoke<SessionLogEntry[]>("nurture_session_log", { udid });
}

/// Which phones have history, and their last line.
export async function nurtureSessionLogSummary() {
  return invoke<SessionLogSummary[]>("nurture_session_log_summary");
}

/// Every comment a session *considered*, newest first — sent, gate-rejected and skipped.
///
/// The command has existed since the audit table did; nothing in the app called it, so the
/// whole record was visible only from the `live_nurture_android` binary's final dump. That is
/// why `distinctFrames` had to come with a panel: a column no screen reads cannot be checked
/// against a run, and this table is where a run explains why a post got no comment.
/**
 * Tokens and comment counts over the whole attempts table, today and in total.
 *
 * Registered, typed and allowlisted since the cost work, and called by nothing until now — the
 * exact shape the repo already recorded once: *"a number nobody reads cannot be checked."*
 */
export async function nurtureCostSummary() {
  return invoke<NurtureCostSummary>("nurture_cost_summary");
}

export async function nurtureListCommentAttempts(limit?: number) {
  return invoke<NurtureCommentAttempt[]>("nurture_list_comment_attempts", { limit });
}

export async function nurtureClearSessionLog(udid: string) {
  return invoke<void>("nurture_clear_session_log", { udid });
}

export async function nurtureStart(udids: string[], durationMinutes?: number | null) {
  return invoke<string[]>("nurture_start", {
    udids,
    durationMinutes: durationMinutes ?? null,
  });
}

export async function nurtureStop(udids: string[] = []) {
  return invoke<void>("nurture_stop", { udids });
}

export async function interactionParseLinks(rawText: string) {
  return invoke<TikTokLinkLine[]>("interaction_parse_links", { rawText });
}

export async function interactionResolveLinks(rawText: string) {
  return invoke<TikTokLinkLine[]>("interaction_resolve_links", { rawText });
}

/**
 * Plan a campaign without running it, and ask what the fleet could actually carry.
 *
 * Registered in Rust since the feature shipped with no caller at all, while the desktop kept
 * its own TypeScript copy of `partition_actors` to draw the same preview. This is the real
 * planner, so what the operator sees before pressing Chạy ngay is what will run.
 */
export async function interactionPreviewThread(request: ThreadCampaignRequest) {
  return invoke<ThreadPreview>("interaction_preview_thread", { request });
}

/**
 * Read one post's numbers from one phone, and ask what the targets would take.
 *
 * **Slow on purpose, and only on a press.** Likes and comments are two label reads on a page
 * already open; a view count is a navigation — TikTok states a play count only on the author's
 * profile grid, and the grid says nothing about which post a tile is, so each candidate is opened
 * and its caption compared. Timed 24/08/2026: about four and a half minutes per reading for a
 * post near the top of the author's grid, and longer when it sits deeper — on top of a cold
 * start. So `readViews` is the operator's choice, and nothing calls this on a debounce.
 *
 * Takes the same exclusive lease a campaign does, because it drives a real phone.
 */
export async function interactionMeasurePost(
  udid: string,
  target: ResolvedTikTokTarget,
  targets: PostTargets,
  actorCount: number,
  readViews: boolean,
) {
  return invoke<InteractionPostReading>("interaction_measure_post", {
    udid,
    target,
    targets,
    actorCount,
    readViews,
  });
}

export async function interactionStartThread(request: ThreadCampaignRequest) {
  return invoke<{ campaign: InteractionCampaignSummary; queued: boolean }>(
    "interaction_start_thread",
    { request },
  );
}

export async function interactionList(limit = 30) {
  return invoke<InteractionCampaignSummary[]>("interaction_list", { limit });
}

export async function interactionGet(campaignId: string) {
  return invoke<InteractionCampaignDetail | null>("interaction_get", { campaignId });
}

export async function interactionCancel(campaignId: string) {
  return invoke<void>("interaction_cancel", { campaignId });
}

/// Omit `assignmentIds` to retry every message that is still retryable.
/// Messages already sent — or whose delivery is unproven — are excluded either
/// way: tapping Send is not idempotent.
export async function interactionRetry(campaignId: string, assignmentIds?: string[]) {
  return invoke<void>("interaction_retry", { campaignId, assignmentIds });
}

export interface InteractionArtifactRecord {
  id: string;
  assignmentId: string | null;
  kind: string;
  relativePath: string | null;
  sha256: string;
  createdAt: string;
}

export interface InteractionArtifactPayload {
  id: string;
  kind: string;
  mimeType: string;
  base64: string;
}

/**
 * What the web lookup learned about every target of one campaign.
 *
 * **The call that stops this being another dead column.** AGENTS.md 9.103 §4: a command can be
 * registered and allowlisted for weeks and still be unreadable, because nothing here invokes it.
 */
export async function interactionListTargetNotes(campaignId: string) {
  return invoke<InteractionTargetNote[]>("interaction_list_target_notes", { campaignId });
}

export async function interactionListArtifacts(campaignId: string) {
  return invoke<InteractionArtifactRecord[]>("interaction_list_artifacts", { campaignId });
}

export async function interactionReadArtifact(artifactId: string) {
  return invoke<InteractionArtifactPayload>("interaction_read_artifact", { artifactId });
}

export async function publicCleanupPreflight(
  campaignId: string,
  assignmentId: string | null,
  kind: PublicCleanupKind,
) {
  return invoke<PublicCleanupCapability>("public_cleanup_preflight", {
    request: { campaignId, assignmentId, kind },
  });
}

export async function publicCleanupExecute(
  requestId: string,
  campaignId: string,
  assignmentId: string,
  kind: PublicCleanupKind,
) {
  return invoke<PublicCleanupExecutionReport>("public_cleanup_execute", {
    request: { requestId, campaignId, assignmentId, kind },
  });
}

/**
 * Subscribe to `riviu://event`.
 *
 * Payloads that do not carry a known `type` are dropped here rather than handed on: the
 * channel is shared, and a subscriber written against one variant should not have to defend
 * itself against the others.
 */
export function listenRiviuEvents(handler: (event: AppEvent) => void): Promise<UnlistenFn> {
  return listen("riviu://event", (event) => {
    const parsed = asAppEvent(event.payload);
    if (parsed) handler(parsed);
  });
}

export async function automationList(includeArchived = false) {
  return invoke<AutomationDefinition[]>("automation_list", { includeArchived });
}

export async function automationGet(definitionId: string, revision: number) {
  return invoke<AutomationDefinitionRecord | null>("automation_get", {
    definitionId,
    revision,
  });
}

export async function automationCreate(
  name: string,
  kind: AutomationKind,
  target: TargetRef,
  config: JsonValue,
) {
  return invoke<AutomationDefinitionRecord>("automation_create", { name, kind, target, config });
}

export async function automationRevise(
  definitionId: string,
  expectedRevision: number,
  target: TargetRef,
  config: JsonValue,
) {
  return invoke<AutomationDefinitionRecord>("automation_revise", {
    definitionId,
    expectedRevision,
    target,
    config,
  });
}

export async function automationArchive(definitionId: string) {
  return invoke<void>("automation_archive", { definitionId });
}

export async function automationScheduleList() {
  return invoke<AutomationSchedule[]>("automation_schedule_list");
}

export async function automationScheduleCreate(
  name: string,
  definitionId: string,
  definitionRevision: number,
  enabled: boolean,
  schedule: AutomationScheduleV1,
) {
  return invoke<AutomationSchedule>("automation_schedule_create", {
    name,
    definitionId,
    definitionRevision,
    enabled,
    schedule,
  });
}

export async function automationScheduleUpdate(
  scheduleId: string,
  expectedRevision: number,
  name: string,
  definitionId: string,
  definitionRevision: number,
  enabled: boolean,
  schedule: AutomationScheduleV1,
) {
  return invoke<AutomationSchedule>("automation_schedule_update", {
    scheduleId,
    expectedRevision,
    name,
    definitionId,
    definitionRevision,
    enabled,
    schedule,
  });
}

export async function orchestrationList(includeArchived = false) {
  return invoke<OrchestrationSummary[]>("orchestration_list", { includeArchived });
}

export async function orchestrationGet(id: string, revision: number | null = null) {
  return invoke<OrchestrationRevisionRecord | null>("orchestration_get", { id, revision });
}

export async function orchestrationValidate(document: OrchestrationDocumentV1) {
  return invoke<CompiledOrchestrationV1>("orchestration_validate", { document });
}

export async function orchestrationSaveRevision(
  document: OrchestrationDocumentV1,
  expectedRevision: number | null,
) {
  return invoke<OrchestrationRevisionRecord>("orchestration_save_revision", {
    document,
    expectedRevision,
  });
}

export async function orchestrationArchive(id: string) {
  return invoke<void>("orchestration_archive", { id });
}

export async function orchestrationRun(
  documentId: string,
  revision: number,
  target: TargetRef,
) {
  return invoke<OrchestrationRunDetail>("orchestration_run", { documentId, revision, target });
}

export async function orchestrationListRuns(limit?: number) {
  return invoke<OrchestrationRunRecord[]>("orchestration_list_runs", {
    limit: limit ?? null,
  });
}

export async function orchestrationGetRun(runId: string) {
  return invoke<OrchestrationRunDetail | null>("orchestration_get_run", { runId });
}

export async function orchestrationReconcile(runId: string) {
  return invoke<OrchestrationRunDetail>("orchestration_reconcile", { runId });
}

export async function orchestrationCancelRun(runId: string) {
  return invoke<OrchestrationRunDetail>("orchestration_cancel_run", { runId });
}

export async function flowActionCatalog() {
  return invoke<ActionDefinition[]>("flow_action_catalog");
}

export async function flowList(includeArchived = false) {
  return invoke<FlowSummary[]>("flow_list", { includeArchived });
}

export async function flowGet(id: string, revision: number | null = null) {
  return invoke<FlowRevisionRecord | null>("flow_get", { id, revision });
}

export async function flowValidate(document: FlowDocumentV2) {
  return invoke<CompiledRevision>("flow_validate", { document });
}

export async function flowSaveRevision(
  document: FlowDocumentV2,
  expectedRevision: number | null,
) {
  return invoke<FlowRevisionRecord>("flow_save_revision", {
    document,
    expectedRevision,
  });
}

export async function flowArchive(id: string) {
  return invoke<void>("flow_archive", { id });
}

export async function flowImportLegacy(scriptJson: string) {
  return invoke<LegacyImportResult>("flow_import_legacy", { scriptJson });
}

export async function flowExport(id: string, revision: number | null = null) {
  return invoke<string>("flow_export", { id, revision });
}

export async function flowRun(
  id: string,
  revision: number | null,
  selection: FlowTargetSelection,
) {
  return invoke<FlowRunRecord>("flow_run", { id, revision, selection });
}

export async function flowCancelRun(runId: string) {
  return invoke<void>("flow_cancel_run", { runId });
}

export async function flowRetryAttempt(attemptId: string) {
  return invoke<FlowNodeAttemptRecord>("flow_retry_attempt", { attemptId });
}

export async function flowListRuns(limit = 100) {
  return invoke<FlowRunRecord[]>("flow_list_runs", { limit });
}

export async function flowGetRun(runId: string) {
  return invoke<FlowRunDetail | null>("flow_get_run", { runId });
}

export async function flowCoordinateFrame(udid: string, bundleId: string) {
  return invoke<FlowCoordinateFrame>("flow_coordinate_frame", { udid, bundleId });
}

export async function flowReadArtifact(artifactId: string) {
  return invoke<FlowArtifactPayload>("flow_read_artifact", { artifactId });
}
