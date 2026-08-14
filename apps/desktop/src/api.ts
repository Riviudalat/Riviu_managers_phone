import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AnalyticsSummary,
  AgentRuntimeView,
  AgentSettings,
  AgentStatus,
  AppLibraryItem,
  AppleIdConfig,
  AuthSession,
  DeviceGroup,
  DeviceInfo,
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
  LocalUser,
  MaterialItem,
  OpLog,
  ProxyConfig,
  PublishTask,
  PublishCampaignDetail,
  PublishCampaignRecord,
  PublishFolderManifest,
  ScheduleItem,
  StreamSettings,
  NurtureCommentAttempt,
  NurtureCommentCost,
  NurtureCostSummary,
  NurtureApiTestResult,
  NurtureSessionStatus,
  NurtureSettings,
  InteractionCampaignDetail,
  InteractionCampaignSummary,
  ThreadCampaignRequest,
  ThreadPreview,
  TikTokLinkLine,
  UpdateStatus,
} from "./types";

export async function startupError() {
  return invoke<string | null>("startup_error");
}

export async function listDevices() {
  return invoke<DeviceInfo[]>("list_devices");
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

export async function uninstallApp(udid: string, bundleId: string) {
  return invoke<void>("uninstall_app", { udid, bundleId });
}

export async function installIpaToGroup(groupId: string, path: string) {
  return invoke<GroupInstallResult[]>("install_ipa_to_group", { groupId, path });
}

export async function screenshot(udid: string) {
  return invoke<string>("screenshot", { udid });
}

export async function syslog(udid: string, lines = 80) {
  return invoke<string>("syslog", { udid, lines });
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

export async function deviceHome(udid: string) {
  return invoke<void>("device_home", { udid });
}

export async function deviceKey(udid: string, key: HardwareKey) {
  return invoke<void>("device_key", { udid, key });
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
}) {
  return invoke<void>("group_input", {
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
  });
}

export async function getStreamSettings() {
  return invoke<StreamSettings>("get_stream_settings");
}

export async function setStreamSettings(settings: StreamSettings) {
  return invoke<StreamSettings>("set_stream_settings", { settings });
}

export async function latestFrame(udid: string) {
  return invoke<string | null>("latest_frame", { udid });
}

export async function viewEndpoint() {
  return invoke<string | null>("view_endpoint");
}

export async function viewEnsure(udid: string) {
  return invoke<void>("view_ensure", { udid });
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

export async function resignWda(udid: string) {
  return invoke<string>("resign_wda", { udid });
}

export async function bulkResignWda(udids: string[]) {
  return invoke<string[]>("bulk_resign_wda", { udids });
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
export async function androidUnavailableReason() {
  return invoke<string | null>("android_unavailable_reason");
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

export async function authSession() {
  return invoke<AuthSession>("auth_session");
}

export async function authLogin(email: string, password: string) {
  return invoke<LocalUser>("auth_login", { email, password });
}

export async function authRegister(email: string, password: string) {
  return invoke<LocalUser>("auth_register", { email, password });
}

export async function getDeviceMeta(udid: string) {
  return invoke<DeviceMeta>("get_device_meta", { udid });
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

export async function deleteGroup(id: string) {
  return invoke<void>("delete_group", { id });
}

export async function listProxies() {
  return invoke<ProxyConfig[]>("list_proxies");
}

export async function saveProxy(proxy: ProxyConfig) {
  return invoke<ProxyConfig>("save_proxy", { proxy });
}

export async function deleteProxy(id: string) {
  return invoke<void>("delete_proxy", { id });
}

export async function exportProxyConfig(id: string) {
  return invoke<string>("export_proxy_config", { id });
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

export async function listSchedules() {
  return invoke<ScheduleItem[]>("list_schedules");
}

export async function saveSchedule(schedule: ScheduleItem) {
  return invoke<ScheduleItem>("save_schedule", { schedule });
}

export async function deleteSchedule(id: string) {
  return invoke<void>("delete_schedule", { id });
}

export async function listPublishTasks() {
  return invoke<PublishTask[]>("list_publish_tasks");
}

export async function createPublishTask(
  name: string,
  scriptName: string,
  materialIds: string[],
  udids: string[],
) {
  return invoke<PublishTask>("create_publish_task", {
    name,
    scriptName,
    materialIds,
    udids,
  });
}

export async function publishScanFolder(sourceRoot: string) {
  return invoke<PublishFolderManifest>("publish_scan_folder", { sourceRoot });
}

export async function publishCreateCampaign(
  sourceRoot: string,
  bundleIds: string[],
  udids: string[],
  runAt?: string | null,
) {
  return invoke<PublishCampaignRecord>("publish_create_campaign", {
    sourceRoot,
    bundleIds,
    udids,
    runAt: runAt ?? null,
  });
}

export async function publishList(limit = 50) {
  return invoke<PublishCampaignRecord[]>("publish_list", { limit });
}

export async function publishGet(campaignId: string) {
  return invoke<PublishCampaignDetail | null>("publish_get", { campaignId });
}

export async function publishCancel(campaignId: string) {
  return invoke<void>("publish_cancel", { campaignId });
}

export async function publishPrepare(campaignId: string) {
  return invoke<PublishCampaignDetail>("publish_prepare", { campaignId });
}

export async function publishTransfer(campaignId: string) {
  return invoke<PublishCampaignDetail>("publish_transfer", { campaignId });
}

export async function publishPost(campaignId: string) {
  return invoke<PublishCampaignDetail>("publish_post", { campaignId });
}

export async function listOpLogs(limit = 100) {
  return invoke<OpLog[]>("list_op_logs", { limit });
}

export async function listUsers() {
  return invoke<LocalUser[]>("list_users");
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

export async function nurtureTestApi(udid: string) {
  return invoke<NurtureApiTestResult>("nurture_test_api", { udid });
}

export async function nurtureListCosts(limit = 100) {
  return invoke<NurtureCommentCost[]>("nurture_list_costs", { limit });
}

export async function nurtureListCommentAttempts(limit = 100) {
  return invoke<NurtureCommentAttempt[]>("nurture_list_comment_attempts", { limit });
}

export async function nurtureCostSummary() {
  return invoke<NurtureCostSummary>("nurture_cost_summary");
}

export async function nurtureSessionStatus() {
  return invoke<NurtureSessionStatus[]>("nurture_session_status");
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

export async function interactionPreviewThread(request: ThreadCampaignRequest) {
  return invoke<ThreadPreview>("interaction_preview_thread", { request });
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

export async function interactionListArtifacts(campaignId: string) {
  return invoke<InteractionArtifactRecord[]>("interaction_list_artifacts", { campaignId });
}

export async function interactionReadArtifact(artifactId: string) {
  return invoke<InteractionArtifactPayload>("interaction_read_artifact", { artifactId });
}

export async function interactionOpenOnDevice(udid: string, url: string) {
  return invoke<void>("interaction_open_on_device", { udid, url });
}

export function listenRiviuEvents(handler: (payload: unknown) => void): Promise<UnlistenFn> {
  return listen("riviu://event", (event) => handler(event.payload));
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
